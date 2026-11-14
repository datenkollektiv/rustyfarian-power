//! ESP32 (WROOM / Feather V2) — PIR Deep-Sleep EXT1 Wake Example (ESP-IDF)
//!
//! Sleeps in deep sleep until an HC-SR501 PIR on GPIO25 reports motion or a timer (60 s) fires,
//! then logs the reset reason and wake cause and sleeps again.
//! It exercises `try_sleep`: right after a PIR wake the sensor output is still HIGH for its hold
//! time, so the first `try_sleep` is rejected and the example retries every 50 ms until the PIR
//! releases.
//! The ESP32 hardware cannot reject a held EXT1 pin, so the pins are read right before entry
//! (`ChipPolicy::Esp32`, guard `Always`).
//! It is the ESP-IDF twin of `hal_esp32_deep_sleep_gpio` and prints the same lines.
//!
//! ## Board
//!
//! - **Chip:** ESP32 (Xtensa LX6), ESP32-WROOM-32 module or Adafruit ESP32 Feather V2 (GPIO25 is A1).
//! - **Console:** the USB-UART bridge on UART0.
//! - **Wake pin:** GPIO25 is an RTC pad, so the wake goes through EXT1 and the wake cause is
//!   `WakeCause::Ext1` with bit 25 set.
//!
//! ## Wiring
//!
//! | Signal      | Connection                                       |
//! |:------------|:-------------------------------------------------|
//! | PIR OUT     | GPIO25                                           |
//! | PIR VCC     | 5V                                               |
//! | PIR GND     | GND                                              |
//! | Pull-down   | external 10 kΩ from GPIO25 to GND (recommended)  |
//!
//! The pull-down is the pull *against* the wake level: the wake level is HIGH, so the pin rests LOW.
//! While awake the example sets the internal pull-down on GPIO25.
//! The HC-SR501 output swings to about 3.3 V, so no level shifter is needed.
//!
//! ## Wake timer
//!
//! The timer fallback defaults to 60 000 ms; override it at build time with `ESP32_SLEEP_TIMER_MS`.
//!
//! ## Run
//!
//! ```shell
//! just run idf_esp32_deep_sleep_gpio
//! ```
//!
//! ## Expected output (serial monitor, not yet bench-verified)
//!
//! ```text
//! Reset reason: DeepSleep
//! Wake cause: Ext1 — fired pins: 0x2000000, GPIO25 fired: true
//! GPIO25 level before sleep: High
//! RTC rtc_us=<n>
//! Sleeping: GPIO25 AnyHigh + 60000 ms timer
//! RTC rtc_us=<n>
//! try_sleep rejected (first): <error>, kind=Rejected
//! ```
//!
//! When the PIR releases, `try_sleep` succeeds and the chip sleeps again.
//! After a power-on the first lines read `Reset reason: PowerOn` and `Wake cause: PowerOn`.
//! `RTC rtc_us` is [`EspRtcClock`]: the RTC timer since power-on, which keeps counting through
//! deep sleep, so after a timer wake the first value minus the last value before sleeping is the
//! slept time plus the boot.
//!
//! ## Bench variants
//!
//! `BENCH_VARIANT` at build time selects the wake configuration; the bench names it as
//! `<example>--<variant>` (`just bench-build`), and an unknown value fails the build.
//!
//! | `BENCH_VARIANT` | Wake                        | Pull | Expected                                         |
//! |:----------------|:----------------------------|:-----|:-------------------------------------------------|
//! | unset           | GPIO25 `AnyHigh`            | down | as above                                         |
//! | `anylow`        | GPIO25 `AnyLow`             | up   | GPIO25 to GND wakes with `Ext1` bit 25           |
//! | `anylow-multi`  | GPIO25 + GPIO26 `AnyLow`    | up   | `try_sleep failed: … kind=Unsupported`, then a timer-only sleep |

use esp_idf_hal::delay::FreeRtos;
use esp_idf_hal::gpio::{AnyIOPin, PinDriver, Pull};
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::sys;
use rustyfarian_esp_idf_power::{
    Error, EspResetReasonSource, EspRtcClock, EspSleepManager, EspWakeCauseSource, GpioWakeLevel,
    ResetReasonSource, SleepErrorKind, SleepManager, WakeCause, WakeCauseSource, WakeSource,
    MAX_TIMER_MS,
};

/// PIR output pin; bit position in the wake mask.
const PIR_PIN: u8 = 25;

/// Second wake pin of the `anylow-multi` variant (Feather V2 pin A0).
const SECOND_PIN: u8 = 26;

/// Wake configuration chosen at build time by `BENCH_VARIANT` (`just bench-build NAME--VARIANT`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Variant {
    /// Unset: GPIO25 `AnyHigh` with a pull-down (the default).
    High,
    /// `anylow`: GPIO25 `AnyLow` with a pull-up.
    Low,
    /// `anylow-multi`: GPIO25 + GPIO26 `AnyLow`; `try_sleep` must refuse it (`AnyLowMultiplePins`).
    LowMulti,
}

/// Byte-string equality usable in a const context.
use stoker::bench::{parse_u64, variant_index};

/// The build's wake configuration: `BENCH_VARIANT` at build time (`anylow`, `anylow-multi`),
/// else the high-level wake; an unknown value fails the build.
const VARIANT: Variant =
    match variant_index(option_env!("BENCH_VARIANT"), &["anylow", "anylow-multi"]) {
        None => Variant::High,
        Some(0) => Variant::Low,
        _ => Variant::LowMulti,
    };
/// Wake on a low level (pull-up) instead of a high one (pull-down).
const WAKE_LOW: bool = !matches!(VARIANT, Variant::High);
/// Arm GPIO26 as a second wake pin.
const MULTI: bool = matches!(VARIANT, Variant::LowMulti);
/// Wake pins as printed in the `Sleeping:` line.
const PINS_LABEL: &str = if MULTI { "GPIO25+GPIO26" } else { "GPIO25" };
/// Wake level as printed in the `Sleeping:` line.
const LEVEL_LABEL: &str = if WAKE_LOW { "AnyLow" } else { "AnyHigh" };
/// EXT1 pin mask of the build.
const PIN_MASK: u64 = if MULTI {
    (1 << PIR_PIN) | (1 << SECOND_PIN)
} else {
    1 << PIR_PIN
};

/// Delay between rejected `try_sleep` attempts.
const RETRY_DELAY_MS: u32 = 50;

/// Log a compact count line every this many rejections.
const LOG_EVERY: u32 = 20;

/// Timer fallback in ms: `ESP32_SLEEP_TIMER_MS` at build time, else 60 s.
const TIMER_MS: u64 = match option_env!("ESP32_SLEEP_TIMER_MS") {
    Some(s) => parse_u64(s),
    None => 60_000,
};

// An out-of-range override would fail validation on every boot; fail the build instead.
const _: () = assert!(
    TIMER_MS >= 1 && TIMER_MS <= MAX_TIMER_MS,
    "ESP32_SLEEP_TIMER_MS must be between 1 and MAX_TIMER_MS"
);

fn main() -> anyhow::Result<()> {
    // SAFETY: link_patches satisfies ESP-IDF's requirement that ROM function
    // stubs are patched before any ESP-IDF API is called. Must be the very
    // first call in main().
    esp_idf_hal::sys::link_patches();

    // Capture the wake cause first (ADR-001): a later sleep attempt, even a
    // rejected one, can clear the hardware wake status.
    let wake_cause = EspWakeCauseSource.last_wake_cause();
    println!(
        "Reset reason: {:?}",
        EspResetReasonSource.last_reset_reason()
    );
    match wake_cause {
        WakeCause::Ext1(m) => println!(
            "Wake cause: Ext1 — fired pins: 0x{:x}, GPIO{} fired: {}",
            m.0,
            PIR_PIN,
            m.contains_pin(PIR_PIN)
        ),
        other => println!("Wake cause: {:?}", other),
    }

    // The pull against the wake level; the driver lives until the chip sleeps or restarts,
    // as the loop never exits. Taken once so no other driver can own the pins.
    let _peripherals = Peripherals::take()?;
    // SAFETY: PIR_PIN is a valid ESP32 GPIO and this is the only live driver of it until the
    // chip sleeps or restarts.
    let pull = if WAKE_LOW { Pull::Up } else { Pull::Down };
    let pir = PinDriver::input(unsafe { AnyIOPin::steal(PIR_PIN) }, pull)?;
    println!("GPIO{} level before sleep: {:?}", PIR_PIN, pir.get_level());
    let second = if MULTI {
        // SAFETY: SECOND_PIN is a valid ESP32 GPIO with no other driver, as for PIR_PIN.
        Some(PinDriver::input(
            unsafe { AnyIOPin::steal(SECOND_PIN) },
            pull,
        )?)
    } else {
        None
    };
    if let Some(p2) = &second {
        println!(
            "GPIO{} level before sleep: {:?}",
            SECOND_PIN,
            p2.get_level()
        );
    }
    // The RTC timer keeps counting through deep sleep: the next boot's value minus the last
    // value before sleeping is the slept time plus the boot.
    println!("RTC rtc_us={}", EspRtcClock::now_us());

    // GPIO wake with isolation is refused (IsolationConflict) on every chip.
    let mut manager = EspSleepManager {
        isolate_gpio: false,
    };
    let sources = [
        WakeSource::GpioLevel {
            pin_mask: PIN_MASK,
            level: if WAKE_LOW {
                GpioWakeLevel::AnyLow
            } else {
                GpioWakeLevel::AnyHigh
            },
        },
        WakeSource::Timer {
            duration_ms: TIMER_MS,
        },
    ];

    println!(
        "Sleeping: {} {} + {} ms timer",
        PINS_LABEL, LEVEL_LABEL, TIMER_MS
    );
    println!("RTC rtc_us={}", EspRtcClock::now_us());

    let mut rejected: u32 = 0;
    loop {
        // On success the chip is asleep and this call does not return.
        match manager.try_sleep(&sources) {
            // Only reachable if the platform returned without sleeping.
            Ok(()) => break,
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
                    "try_sleep failed: {e}, kind={:?} — falling back to a timer-only sleep",
                    e.kind()
                );
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
