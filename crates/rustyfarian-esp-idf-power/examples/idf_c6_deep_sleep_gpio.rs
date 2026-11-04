//! ESP32-C6-DevKitC-1 — PIR Deep-Sleep EXT1 Wake Example (ESP-IDF)
//!
//! Sleeps in deep sleep until an HC-SR501 PIR on GPIO2 reports motion or a timer
//! (60 s by default) fires, then logs the wake cause and sleeps again.
//! It exercises `try_sleep`: right after a PIR wake the sensor output is still HIGH
//! for its hold time, so the first `try_sleep` may be rejected and the example retries
//! every 50 ms until the PIR releases.
//! The ESP32-C6 hardware rejects a held EXT1 pin on its own, so there is no software
//! pre-read on this chip (`ChipPolicy::Esp32C6`, guard `Never`).
//!
//! ## Board
//!
//! - **Chip:** ESP32-C6 (RISC-V, single core, LP domain for the RTC pins)
//! - **Board:** Espressif ESP32-C6-DevKitC-1
//! - The onboard WS2812 (GPIO8) and BOOT button (GPIO9) are left alone; both are strapping pins.
//!
//! ## Wiring
//!
//! | Signal      | Connection                                       |
//! |:------------|:-------------------------------------------------|
//! | PIR OUT     | GPIO2                                            |
//! | PIR VCC     | 5V                                               |
//! | PIR GND     | GND                                              |
//! | Pull-down   | external 10 kΩ from GPIO2 to GND (required)      |
//!
//! The pull-down is the pull *against* the wake level: the wake level is HIGH, so the pin rests LOW.
//! The RTC internal pulls are not available in deep sleep on the C6 (EXT1), so the external
//! resistor is required, not optional.
//! The HC-SR501 output swings to about 3.3 V, so no level shifter is needed.
//! Meter the OUT pin first anyway: the C6 pads are not 5 V tolerant.
//!
//! ## HC-SR501 settings
//!
//! Put the jumper in "H" (retrigger) mode.
//! The time-delay pot (minimum about a few seconds) decides how long the output stays HIGH,
//! and therefore how long the rejection loop runs.
//!
//! While awake the example configures GPIO2 as an input with the internal pull-down,
//! so it does not depend on the pad's reset defaults.
//!
//! ## Wake timer
//!
//! The timer fallback defaults to 60 000 ms.
//! Override it at build time, for example for a bench timer check:
//!
//! ```shell
//! C6_SLEEP_TIMER_MS=5000 just build-example idf_c6_deep_sleep_gpio
//! ```
//!
//! ## Run
//!
//! ```shell
//! just run idf_c6_deep_sleep_gpio
//! ```
//!
//! ## Expected output (serial monitor, not yet bench-verified on the C6)
//!
//! ```text
//! Reset reason: PowerOn
//! Wake cause: PowerOn
//! BENCH case=demo tier=idf cycle=0 reset=PowerOn cause=PowerOn mask=0x0 result=boot guard_us=0 drain_us=0
//! GPIO2 level before sleep: Low
//! Sleeping: GPIO2 AnyHigh + 60000 ms timer
//! ```
//!
//! After a PIR wake the first lines read `Reset reason: DeepSleep` and `Wake cause: Ext1 — fired pins: 0x4, GPIO2 fired: true`.
//! A held PIR makes `try_sleep` return `Rejected`, which prints `try_sleep rejected (first)`; a PIR
//! that wakes the chip at once instead is the hardware-decides result for bench case C6-E1.
//! The `BENCH` line format is the one in `docs/bench/c3-devkitm1-sleep-bench-plan-v1.md`;
//! `cycle` stays 0 because the host counts boots.

use esp_idf_hal::delay::FreeRtos;
use esp_idf_hal::gpio::{AnyIOPin, PinDriver, Pull};
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::sys;
use rustyfarian_esp_idf_power::{
    Error, EspResetReasonSource, EspSleepManager, EspWakeCauseSource, GpioWakeLevel,
    ResetReasonSource, SleepErrorKind, SleepManager, WakeCause, WakeCauseSource, WakeSource,
    MAX_TIMER_MS,
};

/// Tier tag in the shared `BENCH` lines.
const TIER: &str = "idf";

/// PIR output pin; EXT1 wake pin on the C6 (LP GPIO0-7).
///
/// GPIO2 is a plain header pin on the DevKitC-1; GPIO4-7 are the JTAG pads and GPIO8/9 are strapping pins.
const PIR_PIN: u8 = 2;

/// Delay between rejected `try_sleep` attempts.
const RETRY_DELAY_MS: u32 = 50;

/// Log a compact count line every this many rejections.
const LOG_EVERY: u32 = 20;

/// Timer fallback in ms: `C6_SLEEP_TIMER_MS` at build time, else 60 s.
const TIMER_MS: u64 = match option_env!("C6_SLEEP_TIMER_MS") {
    Some(s) => parse_u64(s),
    None => 60_000,
};

// An out-of-range override would fail validation on every boot; fail the build instead.
const _: () = assert!(
    TIMER_MS >= 1 && TIMER_MS <= MAX_TIMER_MS,
    "C6_SLEEP_TIMER_MS must be between 1 and MAX_TIMER_MS"
);

/// Decimal parser usable in a const context; fails the build on anything but digits.
const fn parse_u64(s: &str) -> u64 {
    let b = s.as_bytes();
    assert!(!b.is_empty(), "C6_SLEEP_TIMER_MS must not be empty");
    let mut i = 0;
    let mut v: u64 = 0;
    while i < b.len() {
        assert!(
            b[i].is_ascii_digit(),
            "C6_SLEEP_TIMER_MS must be decimal digits"
        );
        v = v * 10 + (b[i] - b'0') as u64;
        i += 1;
    }
    v
}

/// Variant name of a wake cause, without the fired-pin mask.
fn cause_name(cause: WakeCause) -> (&'static str, u64) {
    match cause {
        WakeCause::PowerOn => ("PowerOn", 0),
        WakeCause::Timer => ("Timer", 0),
        WakeCause::Gpio(m) => ("Gpio", m.0),
        WakeCause::Ext1(m) => ("Ext1", m.0),
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
        WakeCause::Ext1(m) => println!(
            "Wake cause: Ext1 — fired pins: 0x{:x}, GPIO{} fired: {}",
            m.0,
            PIR_PIN,
            m.contains_pin(PIR_PIN)
        ),
        other => println!("Wake cause: {:?}", other),
    }
    println!(
        "BENCH case=demo tier={TIER} cycle=0 reset={} cause={} mask=0x{:x} result=boot guard_us=0 drain_us=0",
        reset_str, cause_str, mask
    );

    // Configure GPIO2 as an input with the pull-down instead of relying on the reset
    // defaults; the driver lives until the chip sleeps or restarts, as the loop never exits.
    // Taken once so no other driver can own the pins; the PIR pin is addressed by number.
    let _peripherals = Peripherals::take()?;
    // SAFETY: PIR_PIN is a valid C6 GPIO and this is the only live driver of it until the
    // chip sleeps or restarts.
    let pir = PinDriver::input(unsafe { AnyIOPin::steal(PIR_PIN) }, Pull::Down)?;
    println!("GPIO{} level before sleep: {:?}", PIR_PIN, pir.get_level());

    // GPIO wake with isolation is refused (IsolationConflict) on every chip.
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
                    "BENCH case=demo tier={TIER} cycle=0 reset={} cause={} mask=0x{:x} result=ok guard_us=0 drain_us=0",
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
                FreeRtos::delay_ms(RETRY_DELAY_MS);
            }
            Err(e) => {
                println!(
                    "BENCH case=demo tier={TIER} cycle=0 reset={} cause={} mask=0x{:x} result=err:{:?} guard_us=0 drain_us=0",
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
