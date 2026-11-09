//! Sleep management traits and host-side no-op implementation.
//!
//! # Architecture
//!
//! Deep sleep on ESP32 is asymmetric: [`SleepManager::sleep`] **never returns**
//! on real hardware (and [`SleepManager::try_sleep`] returns only when the
//! hardware rejects the sleep).
//! The firmware restarts from the entry point at the next boot.
//! This is why [`SleepManager`] and [`WakeCauseSource`] are two separate traits —
//! merging them would imply a round-trip that does not exist in hardware.
//!
//! # Persisting state across deep sleep
//!
//! Deep sleep clears all RAM except the RTC domain.
//! To persist a value across a sleep/wake cycle, place a
//! [`crate::Retained`] record in RTC memory and validate it on every boot with
//! [`crate::ResetReasonSource`]; see the [`crate::retain`] module docs.
//!
//! # Contract in short
//!
//! The full rationale is in `docs/adr/001-hal-neutral-sleep-contract.md`; this is what callers need.
//!
//! - [`SleepManager::sleep`] is fire-and-forget: if a wake source is already asserted, the chip
//!   sleeps and wakes again at once.
//! - [`SleepManager::try_sleep`] refuses instead: it returns an error whose kind is
//!   [`SleepErrorKind::Rejected`], and the caller retries once the source has cleared.
//! - Both validate the whole request before touching hardware, so an `InvalidConfig` error
//!   leaves the wake configuration untouched.
//! - `sources` must not be empty, and a [`WakeSource::Timer`] may not exceed [`MAX_TIMER_MS`].
//! - Read the wake cause at boot with [`WakeCauseSource`], before the first `try_sleep`.
//!
//! ```
//! use stoker::{NoopSleepManager, SleepError, SleepManager, WakeSource};
//!
//! let mut mgr = NoopSleepManager::default();
//!
//! // A timer-only sleep is valid.
//! assert!(mgr.sleep(&[WakeSource::Timer { duration_ms: 60_000 }]).is_ok());
//!
//! // An empty slice is an error: the device would never wake.
//! assert_eq!(mgr.sleep(&[]), Err(SleepError::NoWakeSource));
//! ```
//!
//! # Errors
//!
//! [`SleepManager::Error`] is bounded by the [`Error`] trait, which exposes a
//! HAL-neutral [`SleepErrorKind`] through [`Error::kind`].
//! Generic code branches on the kind without knowing the platform error type.
//!
//! | [`SleepError`] variant                                                   | `kind()`        | Typical recovery                                                        |
//! |:-------------------------------------------------------------------------|:----------------|:------------------------------------------------------------------------|
//! | `NoWakeSource`, `MultipleTimers`, `MultipleGpioLevel`                    | `InvalidConfig` | Fix the `sources` slice; retrying unchanged fails again.                |
//! | `GpioMaskZero`, `GpioMaskOutOfRange`, `PinNotBound`                      | `InvalidConfig` | Use wake-capable pins for this chip, bound for the attempt.             |
//! | `TimerTooShort`, `TimerTooLong`                                          | `InvalidConfig` | Clamp the duration into the tier's range.                               |
//! | `ForeignWakePin`                                                         | `InvalidConfig` | `unlisten` the named pad or bind it, then retry.                        |
//! | `IsolationConflict`                                                      | `InvalidConfig` | Construct the manager with `isolate_gpio: false` for GPIO wake sources. |
//! | `AnyLowMultiplePins`                                                     | `Unsupported`   | Name one pin for `AnyLow`, or use `AnyHigh`; the chip cannot wake on any of several low pins. |
//! | `Rejected`                                                               | `Rejected`      | Wait until the wake source clears (e.g. PIR released), then retry.      |
//! | `UartBusy`                                                               | `Platform`      | Stop or flush UART output before sleeping, then retry.                  |
//! | `AttemptOutOfPhase`                                                      | `Platform`      | Adapter bug; log it and fall back to a reset.                           |
//!
//! Platform error types add their own `Platform` cases (for example `EspSleepError::Idf`
//! when an ESP-IDF call fails): log it and fall back to a reset rather than staying awake on battery.
//!
//! ```
//! use stoker::{Error, NoopSleepManager, SleepErrorKind, SleepManager, WakeSource};
//!
//! /// Returns `true` when the caller should retry later (a wake source is still asserted).
//! fn should_retry<S: SleepManager>(mgr: &mut S, sources: &[WakeSource]) -> bool {
//!     match mgr.try_sleep(sources) {
//!         Ok(()) => false,
//!         Err(e) => e.kind() == SleepErrorKind::Rejected,
//!     }
//! }
//!
//! let sources = [WakeSource::Timer { duration_ms: 1_000 }];
//! assert!(should_retry(&mut NoopSleepManager::rejecting(), &sources));
//! assert!(!should_retry(&mut NoopSleepManager::default(), &sources));
//! ```
//!
//! # ESP-IDF implementation
//!
//! The `EspSleepManager` and `EspWakeCauseSource` hardware drivers live in the
//! companion `rustyfarian-esp-idf-power` crate.

pub mod arm;
pub mod attempt;

/// The reason the device woke from sleep, or [`WakeCause::PowerOn`] on cold boot.
///
/// Read this via [`WakeCauseSource::last_wake_cause`] at the start of `main()`
/// to determine whether the device is recovering from a sleep cycle or starting fresh.
///
/// GPIO-related variants are distinct, so callers can tell exactly which wake
/// mechanism fired without inspecting a potentially empty mask:
/// - [`WakeCause::Ext1`] — EXT1 multi-pin wake; carries the fired-pin mask.
/// - [`WakeCause::Ext0`] — EXT0 single-pin wake; no mask available.
/// - [`WakeCause::Gpio`] — deep-sleep GPIO wake (ESP32-C3/C6); carries the fired-pin mask, which may be empty.
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
    /// Woken by the deep-sleep GPIO wakeup source
    /// (`esp_deep_sleep_enable_gpio_wakeup`).
    ///
    /// This source exists on chips with `SOC_GPIO_SUPPORT_DEEPSLEEP_WAKEUP`
    /// (ESP32-C3 and ESP32-C6), not on ESP32-S3.
    /// The EXT1 status register is not used for this wake source.
    ///
    /// The payload is the mask of pins that fired.
    /// The ESP-IDF tier reads it from `esp_sleep_get_gpio_wakeup_status()`;
    /// the esp-hal tier builds it from per-pin `caused_wakeup()`.
    /// The mask can be empty when the tier cannot tell which pin fired.
    Gpio(GpioWakeMask),
    /// Woken by a touch sensor.
    Touch,
    /// Woken by some other cause (ULP, UART, Wi-Fi, Bluetooth, etc.).
    Other,
}

/// Bitmask of GPIO pins that triggered a GPIO wakeup.
///
/// Bit N is set when GPIO N fired.
/// On ESP32-S3, only bits 0–21 (RTC GPIOs) are meaningful for EXT1.
/// Carried by [`WakeCause::Ext1`] and [`WakeCause::Gpio`]; not used by
/// [`WakeCause::Ext0`], which has no fired-pin mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpioWakeMask(pub u64);

impl GpioWakeMask {
    /// Returns `true` if the given pin number is set in this mask.
    ///
    /// Pins at or above 64 cannot be represented and always return `false`.
    pub fn contains_pin(self, pin: u8) -> bool {
        pin < 64 && self.0 & (1u64 << pin) != 0
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
    /// this chip, so `AnyLow` over several pins is rejected on both tiers
    /// ([`SleepError::AnyLowMultiplePins`]).
    /// A single pin wakes as `ESP_EXT1_WAKEUP_ALL_LOW`, which is equivalent for one pin.
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
    /// On the ESP-IDF tier this uses EXT1 wakeup (`esp_sleep_enable_ext1_wakeup_io`)
    /// on ESP32, ESP32-S3 and ESP32-C6, and deep-sleep GPIO wakeup
    /// (`esp_deep_sleep_enable_gpio_wakeup`, GPIO0-5) on ESP32-C2 and ESP32-C3.
    /// Only RTC GPIOs 0–21 are valid on ESP32-S3.
    /// Bits for out-of-range pins are rejected at configuration time.
    ///
    /// # Hardware requirement
    ///
    /// External pull resistors (10–100 kΩ) are **required** on every EXT1 wake pin
    /// (ESP32, ESP32-S3, ESP32-C6) and recommended on ESP32-C2/C3.
    /// On EXT1 chips the RTC internal pull-up/pull-down resistors are unavailable during
    /// deep sleep when `RTC_PERIPH` is powered down (the default).
    /// On ESP32-C2/C3 ESP-IDF enables the internal pull against the wake level at sleep entry.
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

/// Errors returned by the platform-independent wake-source validation.
///
/// Returned by [`validate_gpio_level_source`], [`validate_wake_sources`] and
/// [`NoopSleepManager`]'s [`SleepManager::sleep`] and [`SleepManager::try_sleep`].
/// Platform implementations surface these through their own
/// [`SleepManager::Error`] type, or use this type directly.
/// Implements [`Error`], so [`Error::kind`] classifies every variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SleepError {
    /// A [`WakeSource::GpioLevel`] `pin_mask` was zero.
    GpioMaskZero,
    /// A [`WakeSource::GpioLevel`] `pin_mask` contains bits outside the
    /// chip's RTC GPIO range.
    GpioMaskOutOfRange {
        /// The rejected mask.
        pin_mask: u64,
        /// The chip's valid wake-pin mask the rejected mask was checked against.
        valid_mask: u64,
    },
    /// More than one [`WakeSource::Timer`] was supplied.
    MultipleTimers {
        /// Number of timer sources supplied.
        count: usize,
    },
    /// More than one [`WakeSource::GpioLevel`] was supplied.
    MultipleGpioLevel,
    /// The `sources` slice was empty; at least one wake source is required.
    NoWakeSource,
    /// The hardware refused to sleep because a wake source was already asserted.
    ///
    /// Returned by [`SleepManager::try_sleep`] only.
    Rejected,
    /// A [`WakeSource::Timer`] duration is below the tier's minimum.
    TimerTooShort {
        /// The rejected duration in milliseconds.
        duration_ms: u64,
        /// The smallest duration the tier accepts, in milliseconds.
        min_ms: u64,
    },
    /// A [`WakeSource::Timer`] duration is above the maximum ([`MAX_TIMER_MS`]).
    TimerTooLong {
        /// The rejected duration in milliseconds.
        duration_ms: u64,
        /// The largest duration the tier accepts, in milliseconds.
        max_ms: u64,
    },
    /// A pin set in a [`WakeSource::GpioLevel`] `pin_mask` was not bound for this sleep attempt.
    PinNotBound {
        /// The GPIO number that has no bound pin.
        pin: u8,
    },
    /// A low-power wake pad outside the pins bound for this sleep attempt is
    /// armed (listening); it would wake or reject the sleep and stay armed afterwards.
    ForeignWakePin {
        /// The GPIO number of the foreign pad.
        pin: u8,
    },
    /// A UART transmitter did not drain within the tier's timeout before sleep entry.
    UartBusy {
        /// The UART number.
        uart: u8,
    },
    /// A [`WakeSource::GpioLevel`] source was combined with GPIO isolation.
    IsolationConflict,
    /// A [`WakeSource::GpioLevel`] with [`GpioWakeLevel::AnyLow`] named more than one pin on a
    /// chip that cannot wake on any one of several low pins.
    ///
    /// The original ESP32 raises it on both tiers: its wake hardware (EXT1) combines
    /// several low pads with AND. Raised by
    /// [`SleepAttemptMachine::new`](crate::SleepAttemptMachine::new) where
    /// [`ChipPolicy::any_low_multi_pin`](crate::ChipPolicy::any_low_multi_pin) is `false`.
    AnyLowMultiplePins,
    /// The tier fed the sleep attempt machine an input that did not fit its phase.
    ///
    /// An adapter bug, not a wake-source condition; returned by [`SleepManager::try_sleep`] only.
    AttemptOutOfPhase,
}

impl core::fmt::Display for SleepError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::GpioMaskZero => write!(f, "GpioLevel pin_mask must not be zero"),
            Self::GpioMaskOutOfRange {
                pin_mask,
                valid_mask,
            } => write!(
                f,
                "pin_mask 0x{:016x} contains bits outside the valid wake-pin mask 0x{:016x}",
                pin_mask, valid_mask
            ),
            Self::MultipleTimers { count } => write!(
                f,
                "at most one Timer wake source is supported per sleep call; \
                 {} were provided",
                count
            ),
            Self::MultipleGpioLevel => write!(
                f,
                "at most one GpioLevel wake source is supported per sleep call; \
                 combine multiple pins into a single pin_mask"
            ),
            Self::NoWakeSource => write!(
                f,
                "no wake source provided; at least one Timer or GpioLevel source is required"
            ),
            Self::Rejected => write!(f, "sleep rejected: a wake source was already asserted"),
            Self::TimerTooShort {
                duration_ms,
                min_ms,
            } => write!(
                f,
                "timer duration {} ms is below the minimum of {} ms",
                duration_ms, min_ms
            ),
            Self::TimerTooLong {
                duration_ms,
                max_ms,
            } => write!(
                f,
                "timer duration {} ms exceeds the maximum of {} ms",
                duration_ms, max_ms
            ),
            Self::PinNotBound { pin } => write!(
                f,
                "wake pin GPIO{} is in the pin_mask but was not bound for this sleep attempt",
                pin
            ),
            Self::ForeignWakePin { pin } => write!(
                f,
                "low-power wake pad GPIO{} is armed but not bound for this sleep attempt",
                pin
            ),
            Self::UartBusy { uart } => write!(
                f,
                "UART{} transmitter did not drain before sleep entry",
                uart
            ),
            Self::IsolationConflict => write!(
                f,
                "a GpioLevel wake source requires isolate_gpio: false; \
                 GPIO isolation prevents wake-capable pins from triggering"
            ),
            Self::AnyLowMultiplePins => write!(
                f,
                "AnyLow wake on several pins is not supported on this tier and chip; \
                 name a single pin or use AnyHigh"
            ),
            Self::AttemptOutOfPhase => write!(
                f,
                "sleep attempt aborted: the tier fed an out-of-phase input"
            ),
        }
    }
}

impl core::error::Error for SleepError {}

/// Error type of a [`SleepManager`] implementation.
///
/// Mirrors the `embedded-hal` 1.0 error convention: [`core::fmt::Debug`] plus
/// a HAL-neutral [`kind`](Error::kind).
/// Generic code branches on the kind without naming the platform error type.
///
/// This trait deliberately does **not** require [`core::error::Error`].
/// Generic consumers branch on [`kind`](Error::kind), and the bound would rule
/// out `defmt`-only error types in bare-metal firmware.
/// [`SleepError`] still implements [`core::error::Error`].
pub trait Error: core::fmt::Debug {
    /// Returns the HAL-neutral classification of this error.
    fn kind(&self) -> SleepErrorKind;
}

/// HAL-neutral classification of a [`SleepManager`] error.
///
/// `#[non_exhaustive]`: downstream `match`es must include a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SleepErrorKind {
    /// The request cannot be honoured as written on this chip and tier.
    InvalidConfig,
    /// The chip cannot honour the requested wake-source combination.
    ///
    /// Only [`SleepError::AnyLowMultiplePins`] maps to it.
    Unsupported,
    /// The hardware refused to sleep because a wake source was already asserted.
    Rejected,
    /// A HAL or SDK failure occurred after validation passed.
    Platform,
    /// Catch-all for variants added in the future.
    Other,
}

impl Error for SleepError {
    fn kind(&self) -> SleepErrorKind {
        match self {
            Self::Rejected => SleepErrorKind::Rejected,
            Self::UartBusy { .. } | Self::AttemptOutOfPhase => SleepErrorKind::Platform,
            // Listed one by one so a new variant forces a kind decision here.
            Self::GpioMaskZero
            | Self::GpioMaskOutOfRange { .. }
            | Self::MultipleTimers { .. }
            | Self::MultipleGpioLevel
            | Self::NoWakeSource
            | Self::TimerTooShort { .. }
            | Self::TimerTooLong { .. }
            | Self::PinNotBound { .. }
            | Self::ForeignWakePin { .. }
            | Self::IsolationConflict => SleepErrorKind::InvalidConfig,
            Self::AnyLowMultiplePins => SleepErrorKind::Unsupported,
        }
    }
}

/// Largest [`WakeSource::Timer`] duration accepted, in milliseconds (`u32::MAX`, about 49.7 days).
///
/// Derived from the esp-hal 48-bit timer comparator; the same cap applies to
/// both the ESP-IDF and esp-hal tiers so a source valid on one is valid on the other.
pub const MAX_TIMER_MS: u64 = u32::MAX as u64;

/// Valid deep-sleep wake-pin mask of the original ESP32 (RTC GPIOs for EXT1).
pub const ESP32_WAKE_PIN_MASK: u64 = 0xFF0E00F015;

/// Valid deep-sleep wake-pin mask of the ESP32-S3 (GPIO0-21, EXT1).
pub const ESP32S3_WAKE_PIN_MASK: u64 = (1u64 << 22) - 1;

/// Valid deep-sleep wake-pin mask of the ESP32-C3 (GPIO0-5, `SOC_GPIO_DEEP_SLEEP_WAKE_VALID_GPIO_MASK`).
pub const ESP32C3_WAKE_PIN_MASK: u64 = 0x3F;

/// Valid deep-sleep wake-pin mask of the ESP32-C6 (GPIO0-7).
pub const ESP32C6_WAKE_PIN_MASK: u64 = 0xFF;

/// Validates the `pin_mask` field of a [`WakeSource::GpioLevel`] source.
///
/// Legacy path: it picks the chip mask from a build-script cfg.
/// The tiers use [`attempt::ChipPolicy::wake_pin_mask`] instead, through
/// [`SleepAttemptMachine::new`](attempt::SleepAttemptMachine::new); keep this for existing callers.
///
/// This is a pure function with no hardware or ESP-IDF dependency.
/// It is called by `EspSleepManager::sleep()` before any FFI call, and is
/// exposed here so the validation logic can be covered by host-side unit tests.
///
/// # Errors
///
/// - [`SleepError::GpioMaskZero`] — at least one pin must be specified.
/// - [`SleepError::GpioMaskOutOfRange`] — `pin_mask` contains bits outside the
///   chip's valid wake-pin mask; the error carries that mask as `valid_mask`.
///
/// # Target specificity
///
/// The valid RTC-GPIO mask is chip-specific and selected at compile time from
/// the `esp32` cfg emitted by this crate's `build.rs` (from the target triple):
/// the ESP32 mask for `xtensa-esp32-espidf`, the ESP32-S3 range (0–21) otherwise.
/// Other ESP32 variants (C3, C6, H2) have different RTC GPIO sets and would
/// require a different mask. This crate targets ESP32-S3 (Heltec WiFi LoRa 32 V3)
/// and ESP32 (Adafruit Feather V2).
///
/// **Host/non-ESP builds use the ESP32-S3 mask by design.** No `esp32` cfg is set
/// off-target, so host unit tests exercise the S3 range. This is deliberate, not a
/// fallback bug: the function is only meaningfully invoked on-device, where
/// `build.rs` sets the cfg matching the actual chip. Host tests therefore validate
/// the S3 boundary logic; the ESP32 mask is covered by the on-target build
/// (`just check-esp32`).
pub fn validate_gpio_level_source(pin_mask: u64) -> Result<(), SleepError> {
    #[cfg(esp32)]
    let valid_mask = ESP32_WAKE_PIN_MASK;
    #[cfg(not(esp32))] // Default to S3 logic for S3 chip and for host tests
    let valid_mask = ESP32S3_WAKE_PIN_MASK;

    validate_gpio_level_source_against(pin_mask, valid_mask)
}

/// Validates a [`WakeSource::GpioLevel`] `pin_mask` against an explicit chip mask.
///
/// Pure function; use it with [`ESP32C3_WAKE_PIN_MASK`], [`ESP32C6_WAKE_PIN_MASK`]
/// or any other `valid_mask` when the cfg-selected mask of
/// [`validate_gpio_level_source`] does not apply.
///
/// # Errors
///
/// - [`SleepError::GpioMaskZero`] — `pin_mask` is zero.
/// - [`SleepError::GpioMaskOutOfRange`] — `pin_mask` has bits outside `valid_mask`.
pub fn validate_gpio_level_source_against(
    pin_mask: u64,
    valid_mask: u64,
) -> Result<(), SleepError> {
    if pin_mask == 0 {
        return Err(SleepError::GpioMaskZero);
    }
    if pin_mask & !valid_mask != 0 {
        return Err(SleepError::GpioMaskOutOfRange {
            pin_mask,
            valid_mask,
        });
    }
    Ok(())
}

/// Validates a `sources` slice passed to [`SleepManager::sleep`] or [`SleepManager::try_sleep`].
///
/// This is a pure function with no hardware or ESP-IDF dependency.
/// Called by `EspSleepManager::sleep()` before any FFI call so the same
/// checks are available to any `SleepManager` implementation.
///
/// # Errors
///
/// Checked in this order:
///
/// - [`SleepError::NoWakeSource`] — `sources` is empty.
/// - [`SleepError::MultipleTimers`] — more than one `WakeSource::Timer`;
///   ESP-IDF supports only one timer source.
/// - [`SleepError::MultipleGpioLevel`] — more than one `WakeSource::GpioLevel`;
///   combine multiple pins into one mask.
/// - [`SleepError::TimerTooLong`] — a timer duration exceeds [`MAX_TIMER_MS`].
pub fn validate_wake_sources(sources: &[WakeSource]) -> Result<(), SleepError> {
    if sources.is_empty() {
        return Err(SleepError::NoWakeSource);
    }
    let timer_count = sources
        .iter()
        .filter(|s| matches!(s, WakeSource::Timer { .. }))
        .count();
    if timer_count > 1 {
        return Err(SleepError::MultipleTimers { count: timer_count });
    }
    let gpio_level_count = sources
        .iter()
        .filter(|s| matches!(s, WakeSource::GpioLevel { .. }))
        .count();
    if gpio_level_count > 1 {
        return Err(SleepError::MultipleGpioLevel);
    }
    for source in sources {
        if let WakeSource::Timer { duration_ms } = *source {
            if duration_ms > MAX_TIMER_MS {
                return Err(SleepError::TimerTooLong {
                    duration_ms,
                    max_ms: MAX_TIMER_MS,
                });
            }
        }
    }
    Ok(())
}

/// `DG_PAD_AUTOHOLD_EN` (bit 11) of `RTC_CNTL_DIG_ISO_REG`: the global digital-pad hold.
///
/// Same bit position on ESP32, ESP32-S3, ESP32-C2 and ESP32-C3 (ESP-IDF v5.3.3
/// `rtc_cntl_reg.h` / `gpio_ll`).
pub const DIG_ISO_DG_PAD_AUTOHOLD_EN: u32 = 1 << 11;

/// `DG_PAD_FORCE_UNHOLD` (bit 14) of `RTC_CNTL_DIG_ISO_REG`: forces every digital pad unheld.
///
/// Same bit position as [`DIG_ISO_DG_PAD_AUTOHOLD_EN`] on the same chips.
pub const DIG_ISO_DG_PAD_FORCE_UNHOLD: u32 = 1 << 14;

/// Returns `true` when the global digital-pad hold is effective.
///
/// Pure function over the raw `RTC_CNTL_DIG_ISO_REG` value.
/// The hold is effective when `DG_PAD_AUTOHOLD_EN` is set and `DG_PAD_FORCE_UNHOLD`
/// is clear.
/// While it is effective on ESP32 and ESP32-S3, sleep entry isolates every unheld
/// digital pad and a hardware-rejected deep sleep never returns.
/// All other bits of the register are ignored.
#[must_use]
pub fn digital_hold_effective(dig_iso_reg: u32) -> bool {
    dig_iso_reg & DIG_ISO_DG_PAD_AUTOHOLD_EN != 0 && dig_iso_reg & DIG_ISO_DG_PAD_FORCE_UNHOLD == 0
}

/// Returns `true` when any pin of `pin_mask` is already at its wake level.
///
/// Pure function: `levels` has bit N set when GPIO N reads high.
/// [`GpioWakeLevel::AnyHigh`] asserts when any masked pin is high,
/// [`GpioWakeLevel::AnyLow`] when any masked pin is low.
/// Pins outside `pin_mask` are ignored and an empty mask never asserts.
#[must_use]
pub fn wake_pins_asserted(levels: u64, pin_mask: u64, level: GpioWakeLevel) -> bool {
    match level {
        GpioWakeLevel::AnyHigh => levels & pin_mask != 0,
        GpioWakeLevel::AnyLow => !levels & pin_mask != 0,
    }
}

/// Returns `true` when every pin of a non-empty `pin_mask` is low.
///
/// Pure function: `levels` has bit N set when GPIO N reads high.
/// This is the EXT1 `ALL_LOW` condition; the attempt machine no longer uses it, because
/// multi-pin `AnyLow` is rejected on the original ESP32.
/// Pins outside `pin_mask` are ignored and an empty mask never asserts.
#[must_use]
pub fn wake_pins_all_low(levels: u64, pin_mask: u64) -> bool {
    pin_mask != 0 && levels & pin_mask == 0
}

/// Fired-pin mask of an EXT1 wake, with the armed configuration as a fallback.
///
/// Pure function: `status` is the EXT1 wake status as the tier read it, `armed_mask` and
/// `armed_low` describe the EXT1 configuration armed before the sleep (`armed_mask` 0 when
/// none is known).
/// A non-empty `status` always wins.
/// An empty status is resolved to `armed_mask` only for a single armed pin with a low wake
/// level, the one case where the armed configuration names the pin without guessing: on the
/// original ESP32 a low-level EXT1 wake reads an empty status from `app_main`, and multi-pin
/// `AnyLow` is refused there. Everything else stays empty.
#[must_use]
pub fn ext1_fired_mask(status: u64, armed_mask: u64, armed_low: bool) -> GpioWakeMask {
    if status != 0 {
        GpioWakeMask(status)
    } else if armed_low && armed_mask.count_ones() == 1 {
        GpioWakeMask(armed_mask)
    } else {
        GpioWakeMask(0)
    }
}

/// Controls entry into a low-power sleep mode.
///
/// # Does not return on real hardware
///
/// On ESP32 targets, `sleep()` calls `esp_deep_sleep_start()` (esp-hal:
/// `sleep_deep`), which never returns.
/// The firmware restarts from the entry point at the next boot.
/// On device it returns only with a configuration error.
/// On host (mock) targets, the call is a no-op and returns `Ok(())`.
///
/// # Rejecting sleep
///
/// `try_sleep()` calls `esp_deep_sleep_try_to_start()` (esp-hal:
/// `sleep_deep_with_rejection`).
/// On device `Ok(())` is unreachable; it returns `Err` whose
/// [`Error::kind`] is [`SleepErrorKind::Rejected`] when a wake source was
/// already asserted, so the caller can handle the event instead of sleeping
/// through it.
/// See [`try_sleep`](Self::try_sleep) for the post-rejection retry guarantee
/// (ADR-003 point 1).
///
/// Use [`WakeCauseSource`] at the start of `main()` to determine why the
/// device woke.
pub trait SleepManager {
    /// Error returned when pre-sleep configuration fails.
    ///
    /// An associated type rather than [`SleepError`] because platform
    /// implementations must surface their own failures (FFI error codes,
    /// driver state) alongside validation errors.
    /// [`NoopSleepManager`] uses [`SleepError`]; `EspSleepManager` uses its own
    /// `EspSleepError`.
    type Error: Error;

    /// Configure the requested wake sources and enter sleep.
    ///
    /// Non-rejecting: maps to `esp_deep_sleep_start()` / `sleep_deep`.
    ///
    /// # Errors
    ///
    /// Returns an error if pre-sleep configuration fails, including
    /// [`SleepError::NoWakeSource`] for an empty `sources` slice.
    /// A failure here is operationally critical — do not continue running on
    /// battery if sleep cannot be entered, as the device will drain the battery.
    #[must_use = "sleep configuration errors must be handled — ignoring them may drain the battery"]
    fn sleep(&mut self, sources: &[WakeSource]) -> Result<(), Self::Error>;

    /// Configure the requested wake sources and enter sleep, unless a wake
    /// source is already asserted.
    ///
    /// Required, with no default: a default delegating to [`sleep`](Self::sleep)
    /// would sleep through a held wake pin.
    ///
    /// # Errors
    ///
    /// Returns an error if pre-sleep configuration fails, including
    /// [`SleepError::NoWakeSource`] for an empty `sources` slice.
    /// Returns an error with [`Error::kind`] equal to [`SleepErrorKind::Rejected`]
    /// when a wake source was already asserted and the device did not sleep.
    ///
    /// # Post-rejection retry guarantee (ADR-003 point 1)
    ///
    /// After any error whose [`Error::kind`] is [`SleepErrorKind::Rejected`],
    /// the caller may retry immediately through the same object — no recovery,
    /// re-construction, or pin re-binding step. The retry is a fresh attempt:
    /// the same validation outcomes and error kinds apply, and a
    /// [`WakeSource::Timer`] duration counts from the retry, not from the
    /// rejected attempt. Once the call has returned, every wake pin named in
    /// the rejected request's `pin_mask` is usable as a plain digital input
    /// (on the esp-hal tier, once the `BoundSleep` borrow ends).
    /// Whether wake sources stay armed between rejection and retry is
    /// adapter-internal and not part of this contract.
    ///
    /// # Rejection from a software pre-read
    ///
    /// [`SleepErrorKind::Rejected`] does not always come from the hardware.
    /// On ESP32 and ESP32-S3 a hardware-rejected deep sleep never returns while
    /// the global digital-pad hold is effective (see [`digital_hold_effective`]),
    /// so the adapters read the `GpioLevel` wake pins right before sleep entry
    /// when the hold is effective and return `Rejected` themselves for a pin
    /// already at its wake level (see [`wake_pins_asserted`]).
    /// The global hold is therefore an input to `try_sleep`.
    /// It costs the isolation of every unheld digital pad during sleep; per-pin holds
    /// on RTC pads do not need it.
    /// A pin that reaches its wake level after the read, and non-pin hardware
    /// rejections, can still hang while the hold is effective.
    /// The read happens before sleep entry applies any internal pull, so a wake pin
    /// needs a bias against its wake level when awake, or a floating pin can read as
    /// asserted and keep the retry loop rejecting.
    ///
    /// # What a rejection leaves behind
    ///
    /// After `Rejected`, every pad outside the request's `pin_mask` and every pad
    /// hold you set, RTC or digital, is as you left it; the pads in `pin_mask` are
    /// plain digital inputs again.
    /// The exception is the global digital-pad hold (`gpio_deep_sleep_hold_en`): on
    /// the ESP32-C2/C3 a non-pin rejection with that hold effective may have
    /// isolated the unheld digital-only pads, so reconfigure them.
    /// Console output may be interrupted, and the CPU clock is switched during the
    /// attempt, so keep SPI, I2C and DMA transfers idle across the call.
    #[must_use = "a rejected sleep must be handled — ignoring it leaves the device awake on battery"]
    fn try_sleep(&mut self, sources: &[WakeSource]) -> Result<(), Self::Error>;
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
/// `SleepManager::sleep` and `SleepManager::try_sleep` validate sources with
/// [`validate_wake_sources`] and return `Ok(())` without entering any sleep mode.
/// [`NoopSleepManager::rejecting`] makes `try_sleep` return
/// [`SleepError::Rejected`] after validation, so retry paths are host-testable.
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
/// use stoker::{NoopSleepManager, WakeCause, WakeCauseSource, GpioWakeMask};
///
/// let mut mock = NoopSleepManager::with_cause(WakeCause::Timer);
/// assert_eq!(mock.last_wake_cause(), WakeCause::Timer);
///
/// let mut gpio_mock = NoopSleepManager::with_cause(
///     WakeCause::Ext1(GpioWakeMask(1u64 << 4))
/// );
/// assert!(matches!(gpio_mock.last_wake_cause(), WakeCause::Ext1(_)));
/// ```
///
/// Simulate a sleep that the hardware rejects:
///
/// ```
/// use stoker::{NoopSleepManager, SleepError, SleepManager, WakeSource};
///
/// let mut mock = NoopSleepManager::rejecting();
/// let sources = [WakeSource::Timer { duration_ms: 1_000 }];
/// assert_eq!(mock.try_sleep(&sources), Err(SleepError::Rejected));
/// ```
pub struct NoopSleepManager {
    cause: WakeCause,
    reject: bool,
}

impl Default for NoopSleepManager {
    fn default() -> Self {
        Self {
            cause: WakeCause::PowerOn,
            reject: false,
        }
    }
}

impl NoopSleepManager {
    /// Create a mock that returns the given `WakeCause` from `last_wake_cause()`.
    pub fn with_cause(cause: WakeCause) -> Self {
        Self {
            cause,
            reject: false,
        }
    }

    /// Create a mock whose `try_sleep` returns [`SleepError::Rejected`].
    ///
    /// Validation errors take precedence over the rejection.
    /// `sleep` still returns `Ok(())`, and the wake cause is [`WakeCause::PowerOn`].
    pub fn rejecting() -> Self {
        Self {
            cause: WakeCause::PowerOn,
            reject: true,
        }
    }
}

impl SleepManager for NoopSleepManager {
    type Error = SleepError;

    fn sleep(&mut self, sources: &[WakeSource]) -> Result<(), SleepError> {
        validate_wake_sources(sources)
    }

    fn try_sleep(&mut self, sources: &[WakeSource]) -> Result<(), SleepError> {
        validate_wake_sources(sources)?;
        if self.reject {
            Err(SleepError::Rejected)
        } else {
            Ok(())
        }
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
    use std::string::ToString;

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
    fn noop_sleep_empty_sources_returns_no_wake_source() {
        let mut mgr = NoopSleepManager::default();
        assert_eq!(mgr.sleep(&[]), Err(SleepError::NoWakeSource));
        assert_eq!(mgr.try_sleep(&[]), Err(SleepError::NoWakeSource));
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
        assert_ne!(WakeCause::Ext0, WakeCause::Gpio(GpioWakeMask(0)));
        assert_ne!(WakeCause::Gpio(GpioWakeMask(0)), WakeCause::Touch);
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
        assert_eq!(err, SleepError::MultipleTimers { count: 2 });
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
        assert_eq!(err, SleepError::MultipleGpioLevel);
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
        assert_eq!(err, SleepError::GpioMaskZero);
        assert!(
            err.to_string().contains("must not be zero"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn validate_gpio_level_source_rejects_bit_22() {
        // Bit 22 is one past the ESP32-S3 RTC GPIO range (0–21).
        let err = validate_gpio_level_source(1u64 << 22).unwrap_err();
        assert_eq!(
            err,
            SleepError::GpioMaskOutOfRange {
                pin_mask: 1u64 << 22,
                valid_mask: (1u64 << 22) - 1,
            }
        );
        assert!(
            err.to_string().contains("outside the valid wake-pin mask"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn validate_gpio_level_source_rejects_high_bits() {
        let err = validate_gpio_level_source(1u64 << 63).unwrap_err();
        assert_eq!(
            err,
            SleepError::GpioMaskOutOfRange {
                pin_mask: 1u64 << 63,
                valid_mask: (1u64 << 22) - 1,
            }
        );
        assert!(
            err.to_string().contains("outside the valid wake-pin mask"),
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

    // --- SleepError tests ---

    #[test]
    fn sleep_error_display_preserves_messages() {
        assert_eq!(
            SleepError::GpioMaskZero.to_string(),
            "GpioLevel pin_mask must not be zero"
        );
        assert_eq!(
            SleepError::GpioMaskOutOfRange {
                pin_mask: 1u64 << 22,
                valid_mask: (1u64 << 22) - 1,
            }
            .to_string(),
            "pin_mask 0x0000000000400000 contains bits outside the valid wake-pin mask 0x00000000003fffff"
        );
        assert_eq!(
            SleepError::MultipleTimers { count: 3 }.to_string(),
            "at most one Timer wake source is supported per sleep call; 3 were provided"
        );
        assert_eq!(
            SleepError::MultipleGpioLevel.to_string(),
            "at most one GpioLevel wake source is supported per sleep call; \
             combine multiple pins into a single pin_mask"
        );
    }

    #[test]
    fn noop_sleep_rejects_with_sleep_error() {
        let mut mgr = NoopSleepManager::default();
        let err = mgr
            .sleep(&[
                WakeSource::Timer { duration_ms: 1 },
                WakeSource::Timer { duration_ms: 2 },
            ])
            .unwrap_err();
        assert_eq!(err, SleepError::MultipleTimers { count: 2 });
    }

    // --- ADR-001 contract tests ---

    #[test]
    fn sleep_error_new_variants_display() {
        assert_eq!(
            SleepError::NoWakeSource.to_string(),
            "no wake source provided; at least one Timer or GpioLevel source is required"
        );
        assert_eq!(
            SleepError::Rejected.to_string(),
            "sleep rejected: a wake source was already asserted"
        );
        assert_eq!(
            SleepError::TimerTooShort {
                duration_ms: 1,
                min_ms: 5
            }
            .to_string(),
            "timer duration 1 ms is below the minimum of 5 ms"
        );
        assert_eq!(
            SleepError::TimerTooLong {
                duration_ms: 10,
                max_ms: 4
            }
            .to_string(),
            "timer duration 10 ms exceeds the maximum of 4 ms"
        );
        assert_eq!(
            SleepError::PinNotBound { pin: 3 }.to_string(),
            "wake pin GPIO3 is in the pin_mask but was not bound for this sleep attempt"
        );
        assert_eq!(
            SleepError::ForeignWakePin { pin: 7 }.to_string(),
            "low-power wake pad GPIO7 is armed but not bound for this sleep attempt"
        );
        assert_eq!(
            SleepError::UartBusy { uart: 1 }.to_string(),
            "UART1 transmitter did not drain before sleep entry"
        );
        let msg = SleepError::IsolationConflict.to_string();
        assert!(msg.contains("GpioLevel"), "unexpected: {msg}");
        assert!(msg.contains("isolate_gpio: false"), "unexpected: {msg}");
        let msg = SleepError::AnyLowMultiplePins.to_string();
        assert!(msg.contains("AnyLow"), "unexpected: {msg}");
        assert!(msg.contains("single pin"), "unexpected: {msg}");
    }

    #[test]
    fn sleep_error_kind_mapping() {
        assert_eq!(SleepError::Rejected.kind(), SleepErrorKind::Rejected);
        assert_eq!(
            SleepError::UartBusy { uart: 0 }.kind(),
            SleepErrorKind::Platform
        );
        assert_eq!(
            SleepError::AttemptOutOfPhase.kind(),
            SleepErrorKind::Platform
        );
        let invalid = [
            SleepError::GpioMaskZero,
            SleepError::GpioMaskOutOfRange {
                pin_mask: 1,
                valid_mask: 0,
            },
            SleepError::MultipleTimers { count: 2 },
            SleepError::MultipleGpioLevel,
            SleepError::NoWakeSource,
            SleepError::TimerTooShort {
                duration_ms: 0,
                min_ms: 1,
            },
            SleepError::TimerTooLong {
                duration_ms: 2,
                max_ms: 1,
            },
            SleepError::PinNotBound { pin: 1 },
            SleepError::ForeignWakePin { pin: 1 },
            SleepError::IsolationConflict,
        ];
        for e in invalid {
            assert_eq!(e.kind(), SleepErrorKind::InvalidConfig, "{e:?}");
        }
        assert_eq!(
            SleepError::AnyLowMultiplePins.kind(),
            SleepErrorKind::Unsupported
        );
    }

    #[test]
    fn sleep_error_kind_is_copy_and_eq() {
        let a = SleepErrorKind::Rejected;
        let b = a;
        assert_eq!(a, b);
        assert_ne!(SleepErrorKind::Unsupported, SleepErrorKind::Other);
        assert_ne!(SleepErrorKind::Platform, SleepErrorKind::InvalidConfig);
    }

    #[test]
    fn validate_wake_sources_precedence() {
        // Duplicates are reported before an oversized timer.
        assert_eq!(
            validate_wake_sources(&[
                WakeSource::Timer {
                    duration_ms: u64::MAX
                },
                WakeSource::Timer { duration_ms: 1 },
            ]),
            Err(SleepError::MultipleTimers { count: 2 })
        );
        // An oversized timer is reported even next to a valid GpioLevel source.
        assert_eq!(
            validate_wake_sources(&[
                WakeSource::GpioLevel {
                    pin_mask: 1u64 << 4,
                    level: GpioWakeLevel::AnyHigh,
                },
                WakeSource::Timer {
                    duration_ms: MAX_TIMER_MS + 1
                },
            ]),
            Err(SleepError::TimerTooLong {
                duration_ms: MAX_TIMER_MS + 1,
                max_ms: MAX_TIMER_MS,
            })
        );
    }

    #[test]
    fn validate_wake_sources_rejects_empty() {
        assert_eq!(validate_wake_sources(&[]), Err(SleepError::NoWakeSource));
    }

    #[test]
    fn validate_wake_sources_timer_max_boundary() {
        assert!(validate_wake_sources(&[WakeSource::Timer {
            duration_ms: MAX_TIMER_MS
        }])
        .is_ok());
        assert_eq!(
            validate_wake_sources(&[WakeSource::Timer {
                duration_ms: MAX_TIMER_MS + 1
            }]),
            Err(SleepError::TimerTooLong {
                duration_ms: MAX_TIMER_MS + 1,
                max_ms: MAX_TIMER_MS
            })
        );
        assert_eq!(
            validate_wake_sources(&[WakeSource::Timer {
                duration_ms: u64::MAX
            }]),
            Err(SleepError::TimerTooLong {
                duration_ms: u64::MAX,
                max_ms: MAX_TIMER_MS
            })
        );
    }

    #[test]
    fn max_timer_ms_is_u32_max() {
        assert_eq!(MAX_TIMER_MS, 4_294_967_295);
    }

    #[test]
    fn gpio_wake_mask_contains_pin_bounds() {
        let mask = GpioWakeMask(u64::MAX);
        assert!(mask.contains_pin(63));
        assert!(!mask.contains_pin(64));
        assert!(!mask.contains_pin(255));
    }

    #[test]
    fn wake_cause_gpio_carries_mask() {
        let cause = WakeCause::Gpio(GpioWakeMask(1u64 << 3));
        let WakeCause::Gpio(mask) = cause else {
            panic!("unexpected variant");
        };
        assert!(mask.contains_pin(3));
        assert!(!mask.contains_pin(2));
        assert_ne!(cause, WakeCause::Gpio(GpioWakeMask(0)));
    }

    #[test]
    fn wake_pin_mask_constants() {
        assert_eq!(ESP32_WAKE_PIN_MASK, 0xFF0E00F015);
        assert_eq!(ESP32S3_WAKE_PIN_MASK, (1u64 << 22) - 1);
        assert_eq!(ESP32C3_WAKE_PIN_MASK, 0x3F);
        assert_eq!(ESP32C6_WAKE_PIN_MASK, 0xFF);
    }

    #[test]
    fn validate_against_c3_mask() {
        assert!(validate_gpio_level_source_against(1 << 5, ESP32C3_WAKE_PIN_MASK).is_ok());
        assert_eq!(
            validate_gpio_level_source_against(1 << 6, ESP32C3_WAKE_PIN_MASK),
            Err(SleepError::GpioMaskOutOfRange {
                pin_mask: 1 << 6,
                valid_mask: 0x3F
            })
        );
        assert_eq!(
            validate_gpio_level_source_against(0, ESP32C3_WAKE_PIN_MASK),
            Err(SleepError::GpioMaskZero)
        );
    }

    #[test]
    fn validate_against_c6_mask() {
        assert!(validate_gpio_level_source_against(1 << 7, ESP32C6_WAKE_PIN_MASK).is_ok());
        assert_eq!(
            validate_gpio_level_source_against(1 << 8, ESP32C6_WAKE_PIN_MASK),
            Err(SleepError::GpioMaskOutOfRange {
                pin_mask: 1 << 8,
                valid_mask: 0xFF
            })
        );
    }

    #[test]
    fn noop_try_sleep_ok() {
        let mut mgr = NoopSleepManager::default();
        assert!(mgr
            .try_sleep(&[WakeSource::Timer { duration_ms: 1000 }])
            .is_ok());
    }

    #[test]
    fn noop_rejecting_try_sleep_returns_rejected() {
        let mut mgr = NoopSleepManager::rejecting();
        assert_eq!(mgr.last_wake_cause(), WakeCause::PowerOn);
        assert_eq!(
            mgr.try_sleep(&[WakeSource::Timer { duration_ms: 1000 }]),
            Err(SleepError::Rejected)
        );
        assert!(mgr
            .sleep(&[WakeSource::Timer { duration_ms: 1000 }])
            .is_ok());
    }

    #[test]
    fn noop_rejecting_validation_errors_win() {
        let mut mgr = NoopSleepManager::rejecting();
        assert_eq!(mgr.try_sleep(&[]), Err(SleepError::NoWakeSource));
        assert_eq!(
            mgr.try_sleep(&[WakeSource::Timer {
                duration_ms: u64::MAX
            }]),
            Err(SleepError::TimerTooLong {
                duration_ms: u64::MAX,
                max_ms: MAX_TIMER_MS
            })
        );
    }

    // Post-rejection retry guarantee (ADR-003 point 1): an immediate bare retry
    // on the same manager — no recovery step in between — rejects again.
    #[test]
    fn noop_rejecting_try_sleep_retry_returns_rejected_again() {
        let mut mgr = NoopSleepManager::rejecting();
        let sources = [WakeSource::Timer { duration_ms: 1000 }];
        assert_eq!(mgr.try_sleep(&sources), Err(SleepError::Rejected));
        assert_eq!(mgr.try_sleep(&sources), Err(SleepError::Rejected));
        let err = mgr.try_sleep(&sources).unwrap_err();
        assert_eq!(err.kind(), SleepErrorKind::Rejected);
    }

    // A rejection latches no state: validation outcomes and precedence on the
    // retry after a `Rejected` attempt are identical to a first attempt.
    #[test]
    fn noop_rejecting_retry_then_invalid_still_validation_error() {
        let mut mgr = NoopSleepManager::rejecting();
        let sources = [WakeSource::Timer { duration_ms: 1000 }];
        assert_eq!(mgr.try_sleep(&sources), Err(SleepError::Rejected));
        assert_eq!(mgr.try_sleep(&[]), Err(SleepError::NoWakeSource));
        assert_eq!(mgr.try_sleep(&sources), Err(SleepError::Rejected));
    }

    // The retry goes through the trait generically — the shape the ADR-003
    // point-4 conformance modules exercise with the real adapters.
    fn kind_of_second_try_sleep<S: SleepManager>(s: &mut S) -> Option<SleepErrorKind> {
        let sources = [WakeSource::Timer { duration_ms: 10 }];
        let _ = s.try_sleep(&sources);
        s.try_sleep(&sources).err().map(|e| e.kind())
    }

    #[test]
    fn generic_consumer_retries_after_rejection() {
        assert_eq!(
            kind_of_second_try_sleep(&mut NoopSleepManager::default()),
            None
        );
        assert_eq!(
            kind_of_second_try_sleep(&mut NoopSleepManager::rejecting()),
            Some(SleepErrorKind::Rejected)
        );
    }

    fn kind_of_try_sleep<S: SleepManager>(s: &mut S) -> Option<SleepErrorKind> {
        s.try_sleep(&[WakeSource::Timer { duration_ms: 10 }])
            .err()
            .map(|e| e.kind())
    }

    #[test]
    fn generic_consumer_branches_on_kind() {
        assert_eq!(kind_of_try_sleep(&mut NoopSleepManager::default()), None);
        assert_eq!(
            kind_of_try_sleep(&mut NoopSleepManager::rejecting()),
            Some(SleepErrorKind::Rejected)
        );
    }

    #[test]
    fn digital_hold_truth_table() {
        let en = DIG_ISO_DG_PAD_AUTOHOLD_EN;
        let un = DIG_ISO_DG_PAD_FORCE_UNHOLD;
        assert!(!digital_hold_effective(0));
        assert!(digital_hold_effective(en));
        assert!(!digital_hold_effective(un));
        assert!(!digital_hold_effective(en | un));
    }

    #[test]
    fn digital_hold_ignores_unrelated_bits() {
        let others = !(DIG_ISO_DG_PAD_AUTOHOLD_EN | DIG_ISO_DG_PAD_FORCE_UNHOLD);
        assert!(!digital_hold_effective(others));
        assert!(digital_hold_effective(others | DIG_ISO_DG_PAD_AUTOHOLD_EN));
        assert!(!digital_hold_effective(!DIG_ISO_DG_PAD_AUTOHOLD_EN));
    }

    #[test]
    fn digital_hold_bit_positions() {
        assert_eq!(DIG_ISO_DG_PAD_AUTOHOLD_EN, 0x0000_0800);
        assert_eq!(DIG_ISO_DG_PAD_FORCE_UNHOLD, 0x0000_4000);
    }

    #[test]
    fn wake_pins_any_high() {
        let mask = 1 << 5;
        assert!(wake_pins_asserted(1 << 5, mask, GpioWakeLevel::AnyHigh));
        assert!(!wake_pins_asserted(0, mask, GpioWakeLevel::AnyHigh));
        assert!(wake_pins_asserted(
            0b11,
            0b11 | 1 << 7,
            GpioWakeLevel::AnyHigh
        ));
    }

    #[test]
    fn wake_pins_any_low() {
        let mask = 1 << 5;
        assert!(wake_pins_asserted(0, mask, GpioWakeLevel::AnyLow));
        assert!(!wake_pins_asserted(1 << 5, mask, GpioWakeLevel::AnyLow));
        let two = (1 << 5) | (1 << 6);
        assert!(wake_pins_asserted(1 << 5, two, GpioWakeLevel::AnyLow));
        assert!(!wake_pins_asserted(two, two, GpioWakeLevel::AnyLow));
    }

    #[test]
    fn wake_pins_outside_mask_ignored() {
        let mask = 1 << 5;
        assert!(!wake_pins_asserted(!mask, mask, GpioWakeLevel::AnyHigh));
        assert!(!wake_pins_asserted(mask, mask, GpioWakeLevel::AnyLow));
    }

    #[test]
    fn wake_pins_empty_mask_never_asserts() {
        for levels in [0, u64::MAX] {
            assert!(!wake_pins_asserted(levels, 0, GpioWakeLevel::AnyHigh));
            assert!(!wake_pins_asserted(levels, 0, GpioWakeLevel::AnyLow));
        }
    }

    #[test]
    fn wake_pins_all_low_cases() {
        let mask = (1 << 5) | (1 << 6);
        assert!(wake_pins_all_low(0, mask));
        assert!(wake_pins_all_low(1 << 7, mask));
        assert!(!wake_pins_all_low(1 << 5, mask));
        assert!(!wake_pins_all_low(mask, mask));
        assert!(!wake_pins_all_low(0, 0));
        assert!(!wake_pins_all_low(u64::MAX, 0));
    }

    #[test]
    fn ext1_fired_mask_status_wins() {
        let status = 1 << 4;
        assert_eq!(ext1_fired_mask(status, 1 << 25, true), GpioWakeMask(status));
        assert_eq!(ext1_fired_mask(status, 0, false), GpioWakeMask(status));
    }

    #[test]
    fn ext1_fired_mask_empty_status_single_low_pin_names_it() {
        assert_eq!(ext1_fired_mask(0, 1 << 25, true), GpioWakeMask(1 << 25));
    }

    #[test]
    fn ext1_fired_mask_empty_status_without_a_unique_low_pin_stays_empty() {
        // AnyHigh: the status is reliable there, so an empty one is not second-guessed.
        assert_eq!(ext1_fired_mask(0, 1 << 25, false), GpioWakeMask(0));
        // Several low pins: the armed mask does not say which one fired.
        assert_eq!(
            ext1_fired_mask(0, (1 << 25) | (1 << 26), true),
            GpioWakeMask(0)
        );
        // No record.
        assert_eq!(ext1_fired_mask(0, 0, true), GpioWakeMask(0));
        assert_eq!(ext1_fired_mask(0, 0, false), GpioWakeMask(0));
    }

    #[test]
    fn ext1_fired_mask_status_wins_over_any_record() {
        // Several armed pins, and a status outside the armed mask: the register is reported as read.
        let armed = (1 << 25) | (1 << 26);
        assert_eq!(ext1_fired_mask(1 << 26, armed, true), GpioWakeMask(1 << 26));
        assert_eq!(ext1_fired_mask(1 << 4, 1 << 25, true), GpioWakeMask(1 << 4));
    }
}
