//! Sleep management traits and host-side no-op implementation.
//!
//! # Architecture
//!
//! Deep sleep on ESP32 is asymmetric: [`SleepManager::sleep`] **never returns**
//! on real hardware.
//! The firmware restarts from the entry point at the next boot.
//! This is why [`SleepManager`] and [`WakeCauseSource`] are two separate traits —
//! merging them would imply a round-trip that does not exist in hardware.
//!
//! # Persisting state across deep sleep
//!
//! Deep sleep clears all RAM except the RTC slow memory.
//! To persist a value across a sleep/wake cycle, place it in the RTC domain:
//!
//! ```rust,ignore
//! #[link_section = ".rtc.data"]
//! static mut BOOT_COUNT: u32 = 0;
//! ```
//!
//! # ESP-IDF implementation
//!
//! Enable the `esp-idf` feature (default) to access `EspSleepManager` and
//! `EspWakeCauseSource` in the `crate::esp_sleep` module.

/// The reason the device woke from sleep, or [`WakeCause::PowerOn`] on cold boot.
///
/// Read this via [`WakeCauseSource::last_wake_cause`] at the start of `main()`
/// to determine whether the device is recovering from a sleep cycle or starting fresh.
///
/// GPIO-related variants are distinct, so callers can tell exactly which wake
/// mechanism fired without inspecting a potentially empty mask:
/// - [`WakeCause::Ext1`] — EXT1 multi-pin wake; carries the fired-pin mask.
/// - [`WakeCause::Ext0`] — EXT0 single-pin wake; no mask available.
/// - [`WakeCause::Gpio`] — ESP32-S3 deep-sleep GPIO wake; mask available separately via `esp_sleep_get_gpio_wakeup_status()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeCause {
    /// Cold power-on or hardware reset — not a wake from sleep.
    ///
    /// Also returned by [`NoopSleepManager`] and whenever the ESP-IDF wakeup
    /// cause is `UNDEFINED`, which occurs on every first boot.
    PowerOn,
    /// Woken by the configured sleep timer.
    Timer,
    /// Woken by an EXT1 GPIO signal (multiple RTC pins, ANY_HIGH or ANY_LOW).
    ///
    /// The payload contains the bitmask of pins that fired, read from
    /// `esp_sleep_get_ext1_wakeup_status()`.
    /// On ESP32-S3, only bits 0–21 (RTC GPIOs) are meaningful.
    Ext1(GpioWakeMask),
    /// Woken by an EXT0 GPIO signal (single RTC pin).
    ///
    /// The EXT1 status register is not used for EXT0 wakeup.
    /// No fired-pin mask is available.
    ///
    /// EXT0 is configured via `esp_sleep_enable_ext0_wakeup()`, which is a
    /// separate ESP-IDF API not currently exposed by this crate.
    /// This variant is returned by [`WakeCauseSource::last_wake_cause`] when the
    /// device wakes from an EXT0 source — for example, if EXT0 was configured
    /// by firmware outside this library's API.
    Ext0,
    /// Woken by the ESP32-S3 deep-sleep GPIO wakeup source
    /// (`esp_deep_sleep_enable_gpio_wakeup`).
    ///
    /// The EXT1 status register is not used for this wake source.
    /// Call `esp_sleep_get_gpio_wakeup_status()` directly if a fired-pin mask
    /// is needed.
    Gpio,
    /// Woken by a touch sensor.
    Touch,
    /// Woken by some other cause (ULP, UART, Wi-Fi, Bluetooth, etc.).
    Other,
}

/// Bitmask of GPIO pins that triggered an EXT1 wakeup.
///
/// Bit N is set when GPIO N fired.
/// On ESP32-S3, only bits 0–21 (RTC GPIOs) are meaningful.
/// Carried by [`WakeCause::Ext1`]; not used by [`WakeCause::Ext0`] or
/// [`WakeCause::Gpio`], which have separate variants precisely because they
/// have no fired-pin mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpioWakeMask(pub u64);

impl GpioWakeMask {
    /// Returns `true` if the given pin number is set in this mask.
    pub fn contains_pin(self, pin: u8) -> bool {
        self.0 & (1u64 << pin) != 0
    }
}

/// The logic level at which an EXT1 GPIO wake source triggers.
///
/// All pins in a [`WakeSource::GpioLevel`] mask share the same level.
/// The semantics are "ANY": wakeup triggers when **any** of the configured
/// pins reach the specified level.
///
/// Corresponds to `ESP_EXT1_WAKEUP_ANY_HIGH` / `ESP_EXT1_WAKEUP_ANY_LOW` in
/// ESP-IDF v5.x.
/// The deprecated `ESP_EXT1_WAKEUP_ALL_LOW` (all-pins-low) mode is not
/// supported by this crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpioWakeLevel {
    /// Wake when **any** pin in the mask goes high.
    ///
    /// Pair with an external pull-down resistor (pin pulled LOW at rest,
    /// external device pulls HIGH to trigger).
    AnyHigh,
    /// Wake when **any** pin in the mask goes low.
    ///
    /// Pair with an external pull-up resistor (pin pulled HIGH at rest,
    /// external device pulls LOW to trigger).
    ///
    /// **ESP32 (original) note:** `ESP_EXT1_WAKEUP_ANY_LOW` does not exist on
    /// this chip; `ESP_EXT1_WAKEUP_ALL_LOW` (all configured pins must be low)
    /// is used instead.
    /// On ESP32-S3 and newer chips the true ANY_LOW semantics are used.
    AnyLow,
}

/// A wake source to configure before entering sleep.
#[derive(Debug, Clone, Copy)]
pub enum WakeSource {
    /// Wake the device after the given number of milliseconds.
    Timer {
        /// Wake delay in milliseconds.
        duration_ms: u64,
    },
    /// Wake the device when one or more GPIO pins reach the configured level.
    ///
    /// Uses ESP-IDF EXT1 wakeup (`esp_sleep_enable_ext1_wakeup_io`).
    /// Only RTC GPIOs 0–21 are valid on ESP32-S3.
    /// Bits for out-of-range pins are rejected at configuration time.
    ///
    /// # Hardware requirement
    ///
    /// External pull resistors (10–100 kΩ) are **required** on every wake pin.
    /// RTC internal pull-up/pull-down resistors are unavailable during deep sleep
    /// when `RTC_PERIPH` is powered down (the default).
    /// A floating pin will have an indeterminate HOLD state and may false-trigger or
    /// fail to trigger — missing external resistors is a common field failure mode.
    ///
    /// Pull direction depends on the chosen level:
    /// - [`GpioWakeLevel::AnyHigh`]: use an external pull-down (pin pulled LOW at rest).
    /// - [`GpioWakeLevel::AnyLow`]: use an external pull-up (pin pulled HIGH at rest).
    ///
    /// # GPIO isolation
    ///
    /// When using GPIO wake sources, set `EspSleepManager { isolate_gpio: false }`.
    /// The default `isolate_gpio: true` may prevent GPIO pins from triggering wakeup.
    GpioLevel {
        /// Bitmask of RTC GPIO pins; bit N = GPIO N.
        ///
        /// Must be non-zero and must not contain bits above position 21 on ESP32-S3.
        pin_mask: u64,
        /// Logic level at which a wake is triggered (ANY semantics — any pin in the
        /// mask reaching this level triggers wake).
        level: GpioWakeLevel,
    },
}

/// Validates the `pin_mask` field of a [`WakeSource::GpioLevel`] source.
///
/// This is a pure function with no hardware or ESP-IDF dependency.
/// It is called by `EspSleepManager::sleep()` before any FFI call, and is
/// exposed here so the validation logic can be covered by host-side unit tests.
///
/// # Errors
///
/// - `pin_mask == 0` — at least one pin must be specified.
/// - `pin_mask` contains bits above position 21 — only RTC GPIOs 0–21 are
///   valid for EXT1 wakeup on ESP32-S3.
///
/// # Target specificity
///
/// The upper-bit check (`pin > 21`) is specific to the ESP32-S3 RTC GPIO
/// range for EXT1.
/// Other ESP32 variants (classic ESP32, C3, C6, H2) have different RTC GPIO
/// sets and would require a different valid mask.
/// This crate targets ESP32-S3 (Heltec WiFi LoRa 32 V3) and ESP32 (Adafruit Feather V2).
#[cfg_attr(not(feature = "esp-idf"), allow(dead_code))]
pub(crate) fn validate_gpio_level_source(pin_mask: u64) -> anyhow::Result<()> {
    if pin_mask == 0 {
        anyhow::bail!("GpioLevel pin_mask must not be zero");
    }

    #[cfg(esp32)]
    let (valid_mask, chip_name, range_desc) = (0xFF0E00F015u64, "ESP32", "");
    #[cfg(not(esp32))] // Default to S3 logic for S3 chip and for host tests
    let (valid_mask, chip_name, range_desc) = ((1u64 << 22) - 1, "ESP32-S3", "0–21");

    if pin_mask & !valid_mask != 0 {
        anyhow::bail!(
            "pin_mask 0x{:016x} contains bits outside RTC GPIO range {} \
             ({} EXT1 limit)",
            pin_mask,
            range_desc,
            chip_name
        );
    }
    Ok(())
}

/// Validates a `sources` slice passed to [`SleepManager::sleep`].
///
/// This is a pure function with no hardware or ESP-IDF dependency.
/// Called by `EspSleepManager::sleep()` before any FFI call so the same
/// checks are available to any `SleepManager` implementation.
///
/// # Errors
///
/// - More than one `WakeSource::Timer` — ESP-IDF supports only one timer source.
/// - More than one `WakeSource::GpioLevel` — combine multiple pins into one mask.
pub(crate) fn validate_wake_sources(sources: &[WakeSource]) -> anyhow::Result<()> {
    let timer_count = sources
        .iter()
        .filter(|s| matches!(s, WakeSource::Timer { .. }))
        .count();
    if timer_count > 1 {
        anyhow::bail!(
            "at most one Timer wake source is supported per sleep call; \
             {} were provided",
            timer_count
        );
    }
    let gpio_level_count = sources
        .iter()
        .filter(|s| matches!(s, WakeSource::GpioLevel { .. }))
        .count();
    if gpio_level_count > 1 {
        anyhow::bail!(
            "at most one GpioLevel wake source is supported per sleep call; \
             combine multiple pins into a single pin_mask"
        );
    }
    Ok(())
}

/// Controls entry into a low-power sleep mode.
///
/// # Does not return on real hardware
///
/// On ESP32 targets, `sleep()` calls `esp_deep_sleep_start()`, which never
/// returns.
/// The firmware restarts from the entry point at the next boot.
/// On host (mock) targets, the call is a no-op and returns `Ok(())`.
///
/// Use [`WakeCauseSource`] at the start of `main()` to determine why the
/// device woke.
pub trait SleepManager {
    /// Configure the requested wake sources and enter sleep.
    ///
    /// # Errors
    ///
    /// Returns an error if pre-sleep configuration fails.
    /// A failure here is operationally critical — do not continue running on
    /// battery if sleep cannot be entered, as the device will drain the battery.
    #[must_use = "sleep configuration errors must be handled — ignoring them may drain the battery"]
    fn sleep(&mut self, sources: &[WakeSource]) -> anyhow::Result<()>;
}

/// Reads the reason the device last woke from sleep.
///
/// This is a separate trait from [`SleepManager`] because [`SleepManager::sleep`]
/// does not return on real hardware.
/// The wake cause must be read at the next boot, not in the same call chain as
/// `sleep()`.
pub trait WakeCauseSource {
    /// Return the cause of the most recent wake from sleep.
    ///
    /// Returns [`WakeCause::PowerOn`] on the first boot (cold power-on) and whenever
    /// the cause cannot be determined.
    fn last_wake_cause(&self) -> WakeCause;
}

/// A no-op sleep manager for host-side unit testing.
///
/// `SleepManager::sleep` validates sources with [`validate_wake_sources`] and
/// returns `Ok(())` without entering any sleep mode.
/// Misconfigured sources (duplicate `Timer`, duplicate `GpioLevel`) are
/// rejected with the same errors as `EspSleepManager`, so tests mirror
/// on-device behaviour.
/// `WakeCauseSource::last_wake_cause` returns the cause configured at construction;
/// defaults to `WakeCause::PowerOn`.
///
/// # Examples
///
/// Use the default (cold-boot path) or program a specific wake cause to test
/// branches in consumer code:
///
/// ```
/// use battery_monitor::{NoopSleepManager, WakeCause, WakeCauseSource, GpioWakeMask};
///
/// let mut mock = NoopSleepManager::with_cause(WakeCause::Timer);
/// assert_eq!(mock.last_wake_cause(), WakeCause::Timer);
///
/// let mut gpio_mock = NoopSleepManager::with_cause(
///     WakeCause::Ext1(GpioWakeMask(1u64 << 4))
/// );
/// assert!(matches!(gpio_mock.last_wake_cause(), WakeCause::Ext1(_)));
/// ```
pub struct NoopSleepManager {
    cause: WakeCause,
}

impl Default for NoopSleepManager {
    fn default() -> Self {
        Self {
            cause: WakeCause::PowerOn,
        }
    }
}

impl NoopSleepManager {
    /// Create a mock that returns the given `WakeCause` from `last_wake_cause()`.
    pub fn with_cause(cause: WakeCause) -> Self {
        Self { cause }
    }
}

impl SleepManager for NoopSleepManager {
    fn sleep(&mut self, sources: &[WakeSource]) -> anyhow::Result<()> {
        validate_wake_sources(sources)
    }
}

impl WakeCauseSource for NoopSleepManager {
    fn last_wake_cause(&self) -> WakeCause {
        self.cause
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_sleep_returns_ok() {
        let mut mgr = NoopSleepManager::default();
        assert!(mgr
            .sleep(&[WakeSource::Timer { duration_ms: 5000 }])
            .is_ok());
    }

    #[test]
    fn noop_sleep_gpio_level_returns_ok() {
        let mut mgr = NoopSleepManager::default();
        assert!(mgr
            .sleep(&[WakeSource::GpioLevel {
                pin_mask: 1u64 << 4,
                level: GpioWakeLevel::AnyLow,
            }])
            .is_ok());
    }

    #[test]
    fn noop_sleep_empty_sources_returns_ok() {
        let mut mgr = NoopSleepManager::default();
        assert!(mgr.sleep(&[]).is_ok());
    }

    #[test]
    fn noop_wake_cause_is_power_on() {
        let mgr = NoopSleepManager::default();
        assert_eq!(mgr.last_wake_cause(), WakeCause::PowerOn);
    }

    #[test]
    fn noop_sleep_manager_with_cause_returns_configured_cause() {
        let mgr = NoopSleepManager::with_cause(WakeCause::Timer);
        assert_eq!(mgr.last_wake_cause(), WakeCause::Timer);
    }

    #[test]
    fn noop_sleep_manager_with_gpio_cause() {
        let mask = GpioWakeMask(1u64 << 7);
        let mgr = NoopSleepManager::with_cause(WakeCause::Ext1(mask));
        assert_eq!(mgr.last_wake_cause(), WakeCause::Ext1(mask));
    }

    #[test]
    fn wake_cause_variants_are_distinct() {
        assert_ne!(WakeCause::PowerOn, WakeCause::Timer);
        assert_ne!(WakeCause::Timer, WakeCause::Ext1(GpioWakeMask(0)));
        assert_ne!(WakeCause::Ext1(GpioWakeMask(0)), WakeCause::Ext0);
        assert_ne!(WakeCause::Ext0, WakeCause::Gpio);
        assert_ne!(WakeCause::Gpio, WakeCause::Touch);
        assert_ne!(WakeCause::Touch, WakeCause::Other);
    }

    #[test]
    fn wake_cause_is_copy() {
        let a = WakeCause::Timer;
        let b = a;
        assert_eq!(a, b);
    }

    #[test]
    fn wake_source_timer_stores_duration() {
        let src = WakeSource::Timer {
            duration_ms: 60_000,
        };
        let WakeSource::Timer { duration_ms } = src else {
            panic!("unexpected variant");
        };
        assert_eq!(duration_ms, 60_000);
    }

    #[test]
    fn gpio_wake_mask_contains_pin() {
        let mask = GpioWakeMask(1u64 << 4 | 1u64 << 7);
        assert!(mask.contains_pin(4));
        assert!(mask.contains_pin(7));
        assert!(!mask.contains_pin(0));
        assert!(!mask.contains_pin(21));
    }

    #[test]
    fn gpio_wake_mask_is_copy() {
        let a = GpioWakeMask(0b1010);
        let b = a;
        assert_eq!(a, b);
    }

    #[test]
    fn gpio_wake_level_variants_are_distinct() {
        assert_ne!(GpioWakeLevel::AnyHigh, GpioWakeLevel::AnyLow);
    }

    // --- validate_wake_sources tests ---

    #[test]
    fn validate_wake_sources_rejects_two_timers() {
        let err = validate_wake_sources(&[
            WakeSource::Timer { duration_ms: 1000 },
            WakeSource::Timer { duration_ms: 2000 },
        ])
        .unwrap_err();
        assert!(
            err.to_string().contains("at most one Timer"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn validate_wake_sources_rejects_two_gpio_level() {
        let err = validate_wake_sources(&[
            WakeSource::GpioLevel {
                pin_mask: 1u64 << 4,
                level: GpioWakeLevel::AnyLow,
            },
            WakeSource::GpioLevel {
                pin_mask: 1u64 << 7,
                level: GpioWakeLevel::AnyHigh,
            },
        ])
        .unwrap_err();
        assert!(
            err.to_string().contains("combine multiple pins"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn validate_wake_sources_accepts_mixed_valid_sources() {
        assert!(validate_wake_sources(&[
            WakeSource::Timer { duration_ms: 5000 },
            WakeSource::GpioLevel {
                pin_mask: 1u64 << 4,
                level: GpioWakeLevel::AnyLow,
            },
        ])
        .is_ok());
    }

    // --- validate_gpio_level_source tests ---

    #[test]
    fn validate_gpio_level_source_rejects_zero_mask() {
        let err = validate_gpio_level_source(0).unwrap_err();
        assert!(
            err.to_string().contains("must not be zero"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn validate_gpio_level_source_rejects_bit_22() {
        // Bit 22 is one past the ESP32-S3 RTC GPIO range (0–21).
        let err = validate_gpio_level_source(1u64 << 22).unwrap_err();
        assert!(
            err.to_string().contains("outside RTC GPIO range"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn validate_gpio_level_source_rejects_high_bits() {
        let err = validate_gpio_level_source(1u64 << 63).unwrap_err();
        assert!(
            err.to_string().contains("outside RTC GPIO range"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn validate_gpio_level_source_accepts_single_pin() {
        assert!(validate_gpio_level_source(1u64 << 4).is_ok());
    }

    #[test]
    fn validate_gpio_level_source_accepts_max_valid_pin() {
        // Bit 21 is the highest valid RTC GPIO on ESP32-S3.
        assert!(validate_gpio_level_source(1u64 << 21).is_ok());
    }

    #[test]
    fn validate_gpio_level_source_accepts_multi_pin_mask() {
        assert!(validate_gpio_level_source(1u64 << 4 | 1u64 << 7 | 1u64 << 21).is_ok());
    }
}
