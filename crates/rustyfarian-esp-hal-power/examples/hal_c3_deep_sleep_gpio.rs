//! ESP32-C3-MINI-1 — PIR Deep-Sleep GPIO Wake Example (esp-hal)
//!
//! Sleeps in deep sleep until an HC-SR501 PIR on GPIO4 reports motion or a timer
//! (60 s by default) fires, then logs the wake cause and sleeps again.
//! It exercises `try_sleep`: right after a PIR wake the sensor output is still HIGH
//! for its hold time, so the first `try_sleep` is rejected and the example retries
//! every 50 ms until the PIR releases.
//! It is the esp-hal twin of `idf_c3_deep_sleep_gpio` and prints the same log lines (`tier=hal`).
//!
//! ## Board
//!
//! - **Chip:** ESP32-C3 (RISC-V, single core), ESP32-C3-MINI-1 module.
//! - **Console:** a CH340 bridge on UART0 (GPIO20/21); `esp-println` (feature `auto`) prints there
//!   unless a USB-Serial-JTAG host is attached.
//! - **Wake pin:** the C3 wakes from deep sleep on GPIO0-5 only, through the per-pin GPIO wake,
//!   so the wake cause is `WakeCause::Gpio` with bit 4 set.
//!
//! ## Wiring
//!
//! | Signal      | Connection                                       |
//! |:------------|:-------------------------------------------------|
//! | PIR OUT     | GPIO4                                            |
//! | PIR VCC     | 5V                                               |
//! | PIR GND     | GND                                              |
//! | Pull-down   | external 10 kΩ from GPIO4 to GND (recommended)   |
//!
//! The pull-down is the pull *against* the wake level: the wake level is HIGH, so the pin rests LOW.
//! The example also enables the internal pull-down; esp-hal copies the pad pulls into the sleep
//! configuration and adds none of its own.
//! The HC-SR501 output swings to about 3.3 V, so no level shifter is needed.
//! Meter the OUT pin first anyway: the C3 pads are not 5 V tolerant.
//!
//! GPIO4 is MTMS (JTAG TMS); make sure no external probe is attached.
//! Put the HC-SR501 jumper in "H" (retrigger) mode: motion while the output is HIGH extends the
//! rejection loop.
//!
//! ## Wake timer
//!
//! The timer fallback defaults to 60 000 ms.
//! Override it at build time, for example for bench case W2:
//!
//! ```shell
//! C3_SLEEP_TIMER_MS=5000 just build-example-hal-c3 hal_c3_deep_sleep_gpio
//! ```
//!
//! The boot `BENCH` line is always printed; set `C3_BENCH_LOG=1` at build time to also print one
//! per `try_sleep` return, which is 20 lines per second while the PIR is held.
//!
//! ## Run
//!
//! ```shell
//! just run-hal-c3 hal_c3_deep_sleep_gpio
//! ```
//!
//! ## Expected output (serial monitor)
//!
//! Built with `C3_BENCH_LOG=1`; without it the per-attempt `BENCH ... result=err:Rejected` lines are absent.
//!
//! ```text
//! Reset reason: CoreDeepSleep (DeepSleep)
//! Wake cause: Gpio — fired pins: 0x10, GPIO4 fired: true
//! BENCH case=demo tier=hal cycle=0 reset=DeepSleep cause=Gpio mask=0x10 result=boot guard_us=0 drain_us=0
//! GPIO4 level before sleep: High
//! Sleeping: GPIO4 AnyHigh + 60000 ms timer
//! try_sleep rejected (first): <error>, kind=Rejected
//! BENCH case=demo tier=hal cycle=0 reset=DeepSleep cause=Gpio mask=0x10 result=err:Rejected guard_us=48 drain_us=3
//! try_sleep rejected: 20 so far
//! ```
//!
//! When the PIR releases, `try_sleep` succeeds and the chip sleeps again; the next boot prints the lines above.
//! After a power-on the first lines read `Wake cause: PowerOn` and `cause=PowerOn mask=0x0`.
//! `guard_us` and `drain_us` are 0 here: they come from the `bench` feature, which this example leaves off.

#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

use esp_hal::{
    delay::Delay,
    gpio::{Input, InputConfig, Pull},
    rtc_cntl::{reset_reason, SocResetReason},
    system::{software_reset, Cpu},
};
use esp_println::println;
use rustyfarian_esp_hal_power::{
    Error, EspHalSleepManager, EspHalWakeCauseSource, GpioWakeLevel, SleepErrorKind, SleepManager,
    WakeCause, WakeCauseSource, WakeSource, MAX_TIMER_MS,
};

/// PIR output pin; bit position in the wake mask.
const PIR_PIN: u8 = 4;

/// Delay between rejected `try_sleep` attempts.
const RETRY_DELAY_MS: u32 = 50;

/// Log a compact count line every this many rejections.
const LOG_EVERY: u32 = 20;

/// Timer fallback in ms: `C3_SLEEP_TIMER_MS` at build time, else 60 s.
const TIMER_MS: u64 = match option_env!("C3_SLEEP_TIMER_MS") {
    Some(s) => parse_u64(s),
    None => 60_000,
};

// An out-of-range override would fail validation on every boot; fail the build instead.
const _: () = assert!(
    TIMER_MS >= 1 && TIMER_MS <= MAX_TIMER_MS,
    "C3_SLEEP_TIMER_MS must be between 1 and MAX_TIMER_MS"
);

/// Print a `BENCH` line for every `try_sleep` return (`C3_BENCH_LOG` set at build time).
///
/// Off by default: while the PIR is held the retry loop would print 20 lines per second.
const BENCH_LOG: bool = option_env!("C3_BENCH_LOG").is_some();

/// Decimal parser usable in a const context; fails the build on anything but digits.
const fn parse_u64(s: &str) -> u64 {
    let b = s.as_bytes();
    assert!(!b.is_empty(), "C3_SLEEP_TIMER_MS must not be empty");
    let mut i = 0;
    let mut v: u64 = 0;
    while i < b.len() {
        assert!(
            b[i].is_ascii_digit(),
            "C3_SLEEP_TIMER_MS must be decimal digits"
        );
        v = v * 10 + (b[i] - b'0') as u64;
        i += 1;
    }
    v
}

/// Short name of the reset reason for the BENCH line.
fn reset_name(reason: Option<SocResetReason>) -> &'static str {
    match reason {
        Some(SocResetReason::ChipPowerOn) => "PowerOn",
        Some(SocResetReason::CoreSw | SocResetReason::Cpu0Sw) => "Software",
        Some(SocResetReason::CoreDeepSleep) => "DeepSleep",
        Some(SocResetReason::SysBrownOut) => "Brownout",
        Some(
            SocResetReason::CoreMwdt0
            | SocResetReason::CoreMwdt1
            | SocResetReason::CoreRtcWdt
            | SocResetReason::Cpu0Mwdt0
            | SocResetReason::Cpu0Mwdt1
            | SocResetReason::Cpu0RtcWdt
            | SocResetReason::SysRtcWdt
            | SocResetReason::SysSuperWdt,
        ) => "Watchdog",
        _ => "Other",
    }
}

fn cause_name(cause: WakeCause) -> (&'static str, u64) {
    match cause {
        WakeCause::PowerOn => ("PowerOn", 0),
        WakeCause::Timer => ("Timer", 0),
        WakeCause::Gpio(mask) => ("Gpio", mask.0),
        WakeCause::Ext1(mask) => ("Ext1", mask.0),
        WakeCause::Ext0 => ("Ext0", 0),
        WakeCause::Touch => ("Touch", 0),
        _ => ("Other", 0),
    }
}

/// Prints the message and restarts: unattended hardware must recover, not hang.
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("PANIC: {}", info);
    Delay::new().delay_millis(50);
    software_reset()
}

#[esp_hal::main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let delay = Delay::new();

    // Configure GPIO4 as an input with the pull against the wake level instead of relying on
    // the reset/JTAG pad defaults. The driver lives until the chip sleeps or restarts.
    let mut pir = Input::new(
        peripherals.GPIO4,
        InputConfig::default().with_pull(Pull::Down),
    );

    // Capture the wake cause first (ADR-001): a later sleep attempt, even a rejected one,
    // overwrites the per-pin wake record.
    let wake_cause = EspHalWakeCauseSource::capture(&[&pir]).last_wake_cause();
    let reset = reset_reason(Cpu::ProCpu);
    let reset_str = reset_name(reset);
    let (cause_str, mask) = cause_name(wake_cause);

    println!("Reset reason: {:?} ({})", reset, reset_str);
    match wake_cause {
        WakeCause::Gpio(m) => println!(
            "Wake cause: Gpio — fired pins: 0x{:x}, GPIO{} fired: {}",
            m.0,
            PIR_PIN,
            m.contains_pin(PIR_PIN)
        ),
        other => println!("Wake cause: {:?}", other),
    }
    println!(
        "BENCH case=demo tier=hal cycle=0 reset={} cause={} mask=0x{:x} result=boot guard_us=0 drain_us=0",
        reset_str, cause_str, mask
    );
    println!("GPIO{} level before sleep: {:?}", PIR_PIN, pir.level());

    let mut manager = EspHalSleepManager::new(peripherals.LPWR);
    let sources = [
        WakeSource::GpioLevel {
            pin_mask: 1 << PIR_PIN,
            level: GpioWakeLevel::AnyHigh,
        },
        WakeSource::Timer {
            duration_ms: TIMER_MS,
        },
    ];

    println!("Sleeping: GPIO{} AnyHigh + {} ms timer", PIR_PIN, TIMER_MS);

    let mut rejected: u32 = 0;
    loop {
        let mut pins = [&mut pir];
        // On success the chip is asleep and this call does not return.
        match manager.bind_wake_pins(&mut pins).try_sleep(&sources) {
            Ok(()) => {
                // Only reachable if the platform returned without sleeping.
                println!(
                    "BENCH case=demo tier=hal cycle=0 reset={} cause={} mask=0x{:x} result=ok guard_us=0 drain_us=0",
                    reset_str, cause_str, mask
                );
                break;
            }
            Err(e) if e.kind() == SleepErrorKind::Rejected => {
                rejected += 1;
                if rejected == 1 {
                    println!("try_sleep rejected (first): {e}, kind={:?}", e.kind());
                } else if rejected.is_multiple_of(LOG_EVERY) {
                    println!("try_sleep rejected: {} so far", rejected);
                }
                if BENCH_LOG {
                    println!(
                        "BENCH case=demo tier=hal cycle=0 reset={} cause={} mask=0x{:x} result=err:Rejected guard_us=0 drain_us=0",
                        reset_str, cause_str, mask
                    );
                }
                delay.delay_millis(RETRY_DELAY_MS);
            }
            Err(e) => {
                println!(
                    "BENCH case=demo tier=hal cycle=0 reset={} cause={} mask=0x{:x} result=err:{:?} guard_us=0 drain_us=0",
                    reset_str,
                    cause_str,
                    mask,
                    e.kind()
                );
                println!("try_sleep failed: {e} — falling back to a timer-only sleep");
                // A restart would retry the same failing configuration in a loop, so sleep
                // on the timer alone; sleep() does not return unless that fails too.
                let mut none: [&mut Input<'_>; 0] = [];
                if let Err(e) = manager
                    .bind_wake_pins(&mut none)
                    .sleep(&[WakeSource::Timer {
                        duration_ms: TIMER_MS,
                    }])
                {
                    println!("timer-only sleep failed: {e} — restarting");
                    delay.delay_millis(50);
                    software_reset();
                }
            }
        }
    }

    // try_sleep returned Ok without sleeping; do not stay awake on battery.
    delay.delay_millis(50);
    software_reset()
}
