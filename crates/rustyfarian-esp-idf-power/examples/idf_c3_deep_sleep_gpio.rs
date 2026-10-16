//! ESP32-C3-DevKitM-1 — PIR Deep-Sleep GPIO Wake Example (ESP-IDF)
//!
//! Sleeps in deep sleep until an HC-SR501 PIR on GPIO4 reports motion or a timer
//! (60 s by default) fires, then logs the wake cause and sleeps again.
//! It exercises `try_sleep`: right after a PIR wake the sensor output is still HIGH
//! for its hold time, so the first `try_sleep` is rejected and the example retries
//! every 50 ms until the PIR releases.
//!
//! ## Board
//!
//! - **Chip:** ESP32-C3 (RISC-V, single core)
//! - **Board:** Espressif ESP32-C3-DevKitM-1 (CP2102N bridge on UART0, GPIO20/21)
//! - The onboard SSD1306 (GPIO6/7) and WS2812 (GPIO8) are present but not used.
//!   GPIO8 is a strapping pin and is left alone.
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
//! The HC-SR501 output swings to about 3.3 V, so no level shifter is needed.
//! Meter the OUT pin first anyway: the C3 pads are not 5 V tolerant.
//! The IDF on C2/C3 also enables the internal pull against the wake level at sleep entry
//! (`CONFIG_ESP_SLEEP_GPIO_ENABLE_INTERNAL_RESISTORS=y`, the default).
//!
//! ## GPIO4 is a JTAG pad
//!
//! GPIO4 is MTMS (JTAG TMS), and GPIO4-7 form the JTAG pad group.
//! A PIR on GPIO4 is fine for firmware, but it rules out pad-JTAG debugging while it is wired.
//! On the default eFuse settings the C3 uses its built-in USB-JTAG instead (bench plan, setup risks),
//! so the pads are free; make sure no external probe is attached.
//!
//! ## HC-SR501 settings
//!
//! Put the jumper in "H" (retrigger) mode.
//! The time-delay pot (minimum about a few seconds) decides how long the output stays HIGH,
//! and therefore how long the rejection loop runs.
//! In retrigger mode, motion while the output is HIGH extends the loop.
//!
//! While awake the example configures GPIO4 as an input with the internal pull-down,
//! so it does not depend on the pad's reset (JTAG) defaults.
//!
//! ## Wake timer
//!
//! The timer fallback defaults to 60 000 ms.
//! Override it at build time, for example for bench case W2:
//!
//! ```shell
//! C3_SLEEP_TIMER_MS=5000 just build-example idf_c3_deep_sleep_gpio
//! ```
//!
//! The boot `BENCH` line is always printed; set `C3_BENCH_LOG=1` at build time to also print one
//! per `try_sleep` return (bench cases H1 and H4), which is 20 lines per second while the PIR is held.
//!
//! ## Run
//!
//! ```shell
//! just run idf_c3_deep_sleep_gpio
//! ```
//!
//! ## Expected output (serial monitor)
//!
//! Built with `C3_BENCH_LOG=1`; without it the per-attempt `BENCH ... result=err:Rejected` lines are absent.
//!
//! ```text
//! Reset reason: DeepSleep
//! Wake cause: Gpio — fired pins: 0x10, GPIO4 fired: true
//! BENCH case=demo tier=idf cycle=0 reset=DeepSleep cause=Gpio mask=0x10 result=boot guard_us=0 drain_us=0
//! Sleeping: GPIO4 AnyHigh + 60000 ms timer
//! try_sleep rejected (first): <error>, kind=Rejected
//! BENCH case=demo tier=idf cycle=0 reset=DeepSleep cause=Gpio mask=0x10 result=err:Rejected guard_us=0 drain_us=0
//! try_sleep rejected: 20 so far
//! ```
//!
//! When the PIR releases, `try_sleep` succeeds and the chip sleeps again; the next boot prints the lines above.
//! After a power-on the first lines read `Wake cause: PowerOn` and `cause=PowerOn mask=0x0`.
//! The `BENCH` line format is the one in `docs/bench/c3-devkitm1-sleep-bench-plan-v1.md`;
//! `cycle` stays 0 because the host counts boots.

use esp_idf_hal::delay::FreeRtos;
use esp_idf_hal::gpio::{PinDriver, Pull};
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::sys;
use rustyfarian_esp_idf_power::{
    Error, EspResetReasonSource, EspSleepManager, EspWakeCauseSource, GpioWakeLevel,
    ResetReasonSource, SleepErrorKind, SleepManager, WakeCause, WakeCauseSource, WakeSource,
    MAX_TIMER_MS,
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

fn main() -> anyhow::Result<()> {
    // SAFETY: link_patches satisfies ESP-IDF's requirement that ROM function
    // stubs are patched before any ESP-IDF API is called. Must be the very
    // first call in main().
    esp_idf_hal::sys::link_patches();

    // Capture the wake cause first (ADR-001): a later sleep attempt, even a
    // rejected one, can clear the hardware wake status.
    let wake_cause = EspWakeCauseSource.last_wake_cause();
    let reset = EspResetReasonSource.last_reset_reason();
    let reset_str = format!("{reset:?}");
    let (cause_str, mask) = cause_name(wake_cause);

    println!("Reset reason: {reset:?}");
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
        "BENCH case=demo tier=idf cycle=0 reset={} cause={} mask=0x{:x} result=boot guard_us=0 drain_us=0",
        reset_str, cause_str, mask
    );

    // Configure GPIO4 as an input with the pull against the wake level instead of
    // relying on the reset/JTAG pad defaults; the arming call only sets wake registers.
    // The driver lives until the chip sleeps or restarts, as the loop never exits.
    let peripherals = Peripherals::take()?;
    let pir = PinDriver::input(peripherals.pins.gpio4, Pull::Down)?;
    println!("GPIO{} level before sleep: {:?}", PIR_PIN, pir.get_level());

    // GPIO wake with isolation is refused (IsolationConflict) until bench case I4.
    let mut manager = EspSleepManager {
        isolate_gpio: false,
    };
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
        // On success the chip is asleep and this call does not return.
        match manager.try_sleep(&sources) {
            Ok(()) => {
                // Only reachable if the platform returned without sleeping.
                println!(
                    "BENCH case=demo tier=idf cycle=0 reset={} cause={} mask=0x{:x} result=ok guard_us=0 drain_us=0",
                    reset_str, cause_str, mask
                );
                break;
            }
            Err(e) if e.kind() == SleepErrorKind::Rejected => {
                rejected += 1;
                if rejected == 1 {
                    println!("try_sleep rejected (first): {e}, kind={:?}", e.kind());
                } else if rejected % LOG_EVERY == 0 {
                    println!("try_sleep rejected: {} so far", rejected);
                }
                if BENCH_LOG {
                    println!(
                    "BENCH case=demo tier=idf cycle=0 reset={} cause={} mask=0x{:x} result=err:Rejected guard_us=0 drain_us=0",
                        reset_str, cause_str, mask
                    );
                }
                FreeRtos::delay_ms(RETRY_DELAY_MS);
            }
            Err(e) => {
                println!(
                    "BENCH case=demo tier=idf cycle=0 reset={} cause={} mask=0x{:x} result=err:{:?} guard_us=0 drain_us=0",
                    reset_str,
                    cause_str,
                    mask,
                    e.kind()
                );
                println!("try_sleep failed: {e} — falling back to a timer-only sleep");
                // A restart would retry the same failing configuration in a loop, so sleep
                // on the timer alone; sleep() does not return unless that fails too.
                if let Err(e) = manager.sleep(&[WakeSource::Timer {
                    duration_ms: TIMER_MS,
                }]) {
                    println!("timer-only sleep failed: {e} — restarting");
                    // SAFETY: esp_restart never returns; staying awake would drain the battery.
                    unsafe { sys::esp_restart() };
                }
            }
        }
    }

    // try_sleep returned Ok without sleeping; do not stay awake on battery.
    // SAFETY: esp_restart never returns.
    unsafe { sys::esp_restart() };
    #[allow(unreachable_code)]
    Ok(())
}
