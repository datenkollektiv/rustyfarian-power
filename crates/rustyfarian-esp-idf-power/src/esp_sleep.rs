//! ESP-IDF deep sleep implementation.
//!
//! Provides [`EspSleepManager`] and [`EspWakeCauseSource`] for ESP32 targets.
//!
//! # GPIO isolation
//!
//! [`EspSleepManager`] calls `esp_sleep_config_gpio_isolate()` automatically
//! before entering deep sleep when `isolate_gpio` is `true` (the default).
//! This prevents floating digital GPIO pins from leaking current during sleep —
//! one of the most commonly missed power optimisations on real boards.
//!
//! When configuring GPIO wake sources, set `isolate_gpio: false` if you need
//! explicit control over pin hold state around wake-capable pins.
//!
//! # Rejection-aware sleep
//!
//! [`SleepManager::try_sleep`] enters deep sleep via `esp_deep_sleep_try_to_start()`
//! and returns [`EspSleepError::Idf`] with `kind() == SleepErrorKind::Rejected` when a
//! wake source is already asserted, for example a PIR wake pin held at its wake level.
//! [`SleepManager::sleep`] keeps `esp_deep_sleep_start()`, which sleeps and wakes again
//! at once in that case.
//!
//! # GPIO wake sources and external pull resistors
//!
//! When using [`WakeSource::GpioLevel`], external pull resistors (10–100 kΩ)
//! are **required** on every wake-capable GPIO pin.
//!
//! RTC internal pull-up/pull-down resistors are unavailable when `RTC_PERIPH`
//! is powered down (the default for deep sleep).
//! Pins left floating will have an indeterminate HOLD state and may false-trigger
//! or fail to trigger — missing external resistors are a common field failure mode.
//!
//! Pull direction depends on the configured level:
//! - `GpioWakeLevel::AnyHigh`: use an external pull-down (pin LOW at rest).
//! - `GpioWakeLevel::AnyLow`: use an external pull-up (pin HIGH at rest).
//!
//! Additionally, set `EspSleepManager { isolate_gpio: false }` when using GPIO
//! wake sources — GPIO isolation may prevent wake-capable pins from triggering.
//!
//! # Persisting state across deep sleep
//!
//! Deep sleep clears all RAM except the RTC slow memory.
//! Place variables in the RTC domain to retain them across a sleep/wake cycle:
//!
//! ```rust,ignore
//! #[link_section = ".rtc.data"]
//! static mut BOOT_COUNT: u32 = 0;
//! ```
//!
//! # Usage
//!
//! Read the wake cause first, before any peripheral initialisation:
//!
//! ```rust,ignore
//! use rustyfarian_esp_idf_power::{EspWakeCauseSource, EspSleepManager, WakeCause, WakeCauseSource,
//!                                 SleepManager, WakeSource};
//!
//! fn main() {
//!     // EspWakeCauseSource is a unit struct — construct it as a value, then call the trait method.
//!     let wake_src = EspWakeCauseSource;
//!     let cause = wake_src.last_wake_cause();
//!     match cause {
//!         WakeCause::PowerOn => log::info!("Cold boot"),
//!         WakeCause::Timer  => log::info!("Woke from timer"),
//!         _                 => log::info!("Other wake: {:?}", cause),
//!     }
//!
//!     // ... do work ...
//!
//!     EspSleepManager::default()
//!         .sleep(&[WakeSource::Timer { duration_ms: 10_000 }])
//!         .expect("sleep configuration failed");
//!
//!     unreachable!("esp_deep_sleep_start() never returns");
//! }
//! ```

use esp_idf_hal::sys;

use stoker::sleep::{
    validate_gpio_level_source, validate_wake_sources, Error, GpioWakeLevel, GpioWakeMask,
    SleepError, SleepErrorKind, SleepManager, WakeCause, WakeCauseSource, WakeSource, MAX_TIMER_MS,
};

/// Error returned by [`EspSleepManager`].
///
/// Implements [`stoker::sleep::Error`], so generic code branches on
/// [`kind()`][Error::kind], and `core::error::Error`, so `?` into `anyhow::Error`
/// still works in firmware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EspSleepError {
    /// The request was rejected by validation before any ESP-IDF call.
    ///
    /// Nothing was configured; the device did not sleep.
    Config(SleepError),
    /// An ESP-IDF call failed, or [`try_sleep`][SleepManager::try_sleep] was rejected.
    ///
    /// `kind` is decided by the call site, never by `code`:
    /// `ESP_ERR_SLEEP_REJECT` aliases `ESP_ERR_INVALID_STATE` (`0x103`), so only the
    /// return of `esp_deep_sleep_try_to_start()` maps to [`SleepErrorKind::Rejected`];
    /// every wake-source configuration failure is [`SleepErrorKind::Platform`].
    Idf {
        /// The raw `esp_err_t` returned by ESP-IDF.
        code: sys::esp_err_t,
        /// The classification of this failure.
        kind: SleepErrorKind,
    },
}

impl From<SleepError> for EspSleepError {
    fn from(err: SleepError) -> Self {
        Self::Config(err)
    }
}

impl core::fmt::Display for EspSleepError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Config(err) => write!(f, "{err}"),
            Self::Idf {
                code,
                kind: SleepErrorKind::Rejected,
            } => write!(
                f,
                "deep sleep rejected: a wake source was already asserted (ESP-IDF error: 0x{:08x})",
                code
            ),
            Self::Idf { code, .. } => write!(f, "ESP-IDF error: 0x{:08x}", code),
        }
    }
}

impl core::error::Error for EspSleepError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Config(err) => Some(err),
            Self::Idf { .. } => None,
        }
    }
}

impl Error for EspSleepError {
    fn kind(&self) -> SleepErrorKind {
        match self {
            Self::Config(err) => err.kind(),
            Self::Idf { kind, .. } => *kind,
        }
    }
}

/// Converts the `esp_err_t` of a wake-source configuration call into a
/// [`SleepErrorKind::Platform`] error, logging which call failed.
fn check(err: sys::esp_err_t, what: &str) -> Result<(), EspSleepError> {
    if err != sys::ESP_OK {
        log::error!("Sleep: {} failed: ESP-IDF error 0x{:08x}", what, err);
        return Err(EspSleepError::Idf {
            code: err,
            kind: SleepErrorKind::Platform,
        });
    }
    Ok(())
}

/// ESP-IDF deep sleep implementation for ESP32 targets.
///
/// Clears any previously configured wake sources, configures the requested
/// sources, optionally isolates GPIO pins, then enters deep sleep via
/// `esp_deep_sleep_start()` ([`sleep`][SleepManager::sleep]) or
/// `esp_deep_sleep_try_to_start()` ([`try_sleep`][SleepManager::try_sleep]).
///
/// **Neither method returns on real hardware when the device sleeps.**
/// The next code to run is the firmware entry point after the device wakes.
///
/// # Construction
///
/// Use [`EspSleepManager::default()`] for the recommended configuration
/// (`isolate_gpio: true`).
/// Set `isolate_gpio: false` if you need manual control over GPIO hold state
/// when configuring GPIO wake sources.
pub struct EspSleepManager {
    /// Isolate all digital GPIO pins before entering sleep.
    ///
    /// When `true` (default), calls `esp_sleep_config_gpio_isolate()` just
    /// before entering deep sleep to prevent floating inputs from leaking
    /// current during sleep.
    pub isolate_gpio: bool,
}

impl Default for EspSleepManager {
    fn default() -> Self {
        Self { isolate_gpio: true }
    }
}

impl EspSleepManager {
    /// Validate `sources`, clear previous wake sources and configure the new ones.
    ///
    /// The whole request is validated before the first ESP-IDF call, so a
    /// [`EspSleepError::Config`] error leaves the wake configuration untouched.
    fn configure(&self, sources: &[WakeSource]) -> Result<(), EspSleepError> {
        validate_wake_sources(sources)?;

        let mut has_gpio_level = false;
        for source in sources {
            if let WakeSource::GpioLevel { pin_mask, .. } = source {
                // The valid wake-pin mask is chip-specific (ESP32 vs ESP32-S3,
                // selected by the `esp32` cfg); see `validate_gpio_level_source`.
                validate_gpio_level_source(*pin_mask)?;
                has_gpio_level = true;
            }
        }

        if has_gpio_level && self.isolate_gpio {
            return Err(SleepError::IsolationConflict.into());
        }

        // Clear any wake sources left over from a previous sleep cycle so the
        // caller gets deterministic behaviour regardless of prior state.
        //
        // SAFETY: esp_sleep_disable_wakeup_source is safe to call at any time
        // before deep sleep entry. Passing ESP_SLEEP_WAKEUP_ALL disables every
        // registered source in one call.
        let ret = unsafe {
            sys::esp_sleep_disable_wakeup_source(sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_ALL)
        };
        check(ret, "clearing previous wake sources")?;

        for source in sources {
            match source {
                WakeSource::Timer { duration_ms } => {
                    // Unreachable after validate_wake_sources caps the duration at
                    // MAX_TIMER_MS; the arithmetic stays checked.
                    let duration_us =
                        duration_ms
                            .checked_mul(1_000)
                            .ok_or(SleepError::TimerTooLong {
                                duration_ms: *duration_ms,
                                max_ms: MAX_TIMER_MS,
                            })?;
                    // SAFETY: esp_sleep_enable_timer_wakeup configures the RTC
                    // timer wakeup source. Safe to call before deep sleep entry;
                    // duration_us has been validated above.
                    let ret = unsafe { sys::esp_sleep_enable_timer_wakeup(duration_us) };
                    check(ret, "configuring timer wakeup")?;
                    log::info!("Sleep: timer wake configured for {}ms", duration_ms);
                }
                WakeSource::GpioLevel { pin_mask, level } => {
                    let mode = match level {
                        GpioWakeLevel::AnyHigh => {
                            sys::esp_sleep_ext1_wakeup_mode_t_ESP_EXT1_WAKEUP_ANY_HIGH
                        }
                        GpioWakeLevel::AnyLow => {
                            // ESP_EXT1_WAKEUP_ANY_LOW is only available on ESP32-S3 and
                            // newer chips. The original ESP32 only exposes ALL_LOW (all
                            // configured pins must be low simultaneously). The semantics
                            // differ, but ALL_LOW is the closest available mode on ESP32.
                            #[cfg(esp32)]
                            {
                                sys::esp_sleep_ext1_wakeup_mode_t_ESP_EXT1_WAKEUP_ALL_LOW
                            }
                            #[cfg(not(esp32))]
                            {
                                sys::esp_sleep_ext1_wakeup_mode_t_ESP_EXT1_WAKEUP_ANY_LOW
                            }
                        }
                    };
                    // SAFETY: esp_sleep_enable_ext1_wakeup_io() configures EXT1 wake
                    // on RTC GPIOs. pin_mask has been validated against the chip's
                    // wake-pin mask and is non-zero. Must be called after disable-all
                    // and before deep sleep entry.
                    let ret = unsafe { sys::esp_sleep_enable_ext1_wakeup_io(*pin_mask, mode) };
                    check(ret, "configuring GPIO EXT1 wakeup")?;
                    log::info!(
                        "Sleep: EXT1 GPIO wake configured (mask: 0x{:x}, level: {:?})",
                        pin_mask,
                        level
                    );
                }
            }
        }

        if self.isolate_gpio {
            // SAFETY: esp_sleep_config_gpio_isolate isolates all digital GPIO
            // pins to prevent floating inputs from leaking current during sleep.
            // Returns void in ESP-IDF v5.x. Must be called after wake sources
            // are configured and before deep sleep entry.
            unsafe {
                sys::esp_sleep_config_gpio_isolate();
            }
        }

        Ok(())
    }
}

impl SleepManager for EspSleepManager {
    type Error = EspSleepError;

    /// Clear previous wake sources, configure new ones, and enter deep sleep.
    ///
    /// All previously registered wake sources are disabled first via
    /// `esp_sleep_disable_wakeup_source(ALL)`, giving callers deterministic
    /// "what I pass in is what I get" semantics across multiple sleep cycles.
    ///
    /// **This function does not return on real hardware** unless configuration fails.
    /// A wake source that is already asserted wakes the device again at once;
    /// use [`try_sleep`][SleepManager::try_sleep] to get an error instead.
    ///
    /// # Errors
    ///
    /// - [`EspSleepError::Config`] — validation failed (no source, more than one
    ///   `Timer` or `GpioLevel`, a timer above [`MAX_TIMER_MS`], a bad pin mask,
    ///   or a `GpioLevel` source with `isolate_gpio: true`).
    /// - [`EspSleepError::Idf`] with kind [`SleepErrorKind::Platform`] — an ESP-IDF
    ///   wake-source configuration call failed.
    /// - The device does not enter sleep if an error is returned.
    fn sleep(&mut self, sources: &[WakeSource]) -> Result<(), EspSleepError> {
        self.configure(sources)?;

        log::info!("Entering deep sleep");

        // SAFETY: All wake sources have been configured above. GPIO pins are
        // isolated (when isolate_gpio is true). esp_deep_sleep_start() never
        // returns — the next execution begins at the firmware entry point when
        // the device wakes.
        unsafe {
            sys::esp_deep_sleep_start();
        }
    }

    /// Like [`sleep`][SleepManager::sleep], but enters deep sleep via
    /// `esp_deep_sleep_try_to_start()`, which returns when the sleep is rejected.
    ///
    /// **This function does not return on real hardware when the device sleeps.**
    ///
    /// # Errors
    ///
    /// - Every error of [`sleep`][SleepManager::sleep].
    /// - [`EspSleepError::Idf`] with kind [`SleepErrorKind::Rejected`] — ESP-IDF
    ///   rejected the sleep because a wake source was already asserted (for example
    ///   a wake pin held at its wake level). The wake sources and the GPIO
    ///   isolation config stay applied; a retry re-runs the whole configuration,
    ///   so call `try_sleep` again once the source has cleared.
    fn try_sleep(&mut self, sources: &[WakeSource]) -> Result<(), EspSleepError> {
        self.configure(sources)?;

        log::info!("Trying to enter deep sleep");

        // SAFETY: All wake sources have been configured above. GPIO pins are
        // isolated (when isolate_gpio is true). esp_deep_sleep_try_to_start()
        // does not return when the device sleeps; it returns only when the sleep
        // request is rejected.
        let code = unsafe { sys::esp_deep_sleep_try_to_start() };
        if code == sys::ESP_OK {
            return Ok(());
        }
        // Mapped by call site: any return from esp_deep_sleep_try_to_start is a
        // rejection. ESP_ERR_SLEEP_REJECT aliases ESP_ERR_INVALID_STATE, so the code
        // alone cannot identify it.
        Err(EspSleepError::Idf {
            code,
            kind: SleepErrorKind::Rejected,
        })
    }
}

/// Reads the wakeup cause from the ESP-IDF wakeup cause register.
///
/// # When to call
///
/// Call [`last_wake_cause`][WakeCauseSource::last_wake_cause] as early as
/// practical in `main()`, before peripheral initialisation.
/// The EXT1 status register is hardware-preserved and survives until the next
/// sleep entry, so calling it later usually works — but reading it early is
/// the safest approach and the one documented by Espressif.
///
/// # Cold boot behaviour
///
/// `esp_sleep_get_wakeup_cause()` returns `ESP_SLEEP_WAKEUP_UNDEFINED` on
/// first power-on (not only after a wake from sleep).
/// This is mapped to [`WakeCause::PowerOn`].
pub struct EspWakeCauseSource;

impl WakeCauseSource for EspWakeCauseSource {
    fn last_wake_cause(&self) -> WakeCause {
        // SAFETY: esp_sleep_get_wakeup_cause() reads the wakeup cause register
        // set by the bootloader. Safe to call at any time; does not modify any
        // hardware state.
        let cause = unsafe { sys::esp_sleep_get_wakeup_cause() };

        #[allow(non_upper_case_globals)]
        match cause {
            sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_UNDEFINED => WakeCause::PowerOn,
            sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_TIMER => WakeCause::Timer,
            sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_EXT1 => {
                // SAFETY: esp_sleep_get_ext1_wakeup_status() reads the EXT1
                // fired-pin bitmask written by hardware before wake.
                // Valid only when the wakeup cause is EXT1; reads a register only
                // and does not modify any hardware state.
                // Recommended to call early in main() before peripheral init,
                // though the EXT1 register is preserved until the next sleep entry.
                let mask = unsafe { sys::esp_sleep_get_ext1_wakeup_status() };
                WakeCause::Ext1(GpioWakeMask(mask))
            }
            sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_EXT0 => {
                // EXT0 uses a single RTC GPIO; the EXT1 status register is not
                // populated. The configured pin is implicit from the wake source
                // that was active at sleep entry.
                WakeCause::Ext0
            }
            sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_GPIO => {
                // Deep-sleep GPIO wakeup (`esp_deep_sleep_enable_gpio_wakeup`), an
                // ESP32-C3/C6 feature (SOC_GPIO_SUPPORT_DEEPSLEEP_WAKEUP), not S3.
                // The EXT1 status register is not used for this source.
                WakeCause::Gpio(gpio_wakeup_mask())
            }
            sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_TOUCHPAD => WakeCause::Touch,
            _ => WakeCause::Other,
        }
    }
}

/// Fired-pin mask of a deep-sleep GPIO wakeup.
///
/// Reads `esp_sleep_get_gpio_wakeup_status()` on chips with deep-sleep GPIO wakeup
/// (ESP32-C3/C6, esp-idf-sys cfg `esp_idf_soc_gpio_support_deepsleep_wakeup`).
#[cfg(esp_idf_soc_gpio_support_deepsleep_wakeup)]
fn gpio_wakeup_mask() -> GpioWakeMask {
    // SAFETY: esp_sleep_get_gpio_wakeup_status() reads the deep-sleep GPIO
    // wakeup status register only and does not modify any hardware state.
    GpioWakeMask(unsafe { sys::esp_sleep_get_gpio_wakeup_status() })
}

/// Fired-pin mask of a GPIO wakeup on chips without deep-sleep GPIO wakeup.
///
/// ESP32 and ESP32-S3 report `ESP_SLEEP_WAKEUP_GPIO` only after light sleep and
/// have no status register for it, so the mask is empty.
#[cfg(not(esp_idf_soc_gpio_support_deepsleep_wakeup))]
fn gpio_wakeup_mask() -> GpioWakeMask {
    GpioWakeMask(0)
}
