//! ESP-IDF deep sleep implementation.
//!
//! Provides [`EspSleepManager`] and [`EspWakeCauseSource`] for ESP32 targets.
//!
//! # GPIO wake mechanism per chip
//!
//! [`WakeSource::GpioLevel`] maps to a different ESP-IDF mechanism per chip:
//!
//! | Chip | Mechanism | Valid wake pins |
//! |---|---|---|
//! | ESP32, ESP32-S3, ESP32-C6 | EXT1 (`esp_sleep_enable_ext1_wakeup_io`, RTC IO) | chip specific, see `validate_gpio_level_source` |
//! | ESP32-C2, ESP32-C3 | deep-sleep GPIO wakeup (`esp_deep_sleep_enable_gpio_wakeup`) | GPIO0-5 (`ESP32C3_WAKE_PIN_MASK`) |
//!
//! The pin mask is validated before any ESP-IDF call.
//! Implementation notes against ESP-IDF v5.3.3 are in
//! `docs/features/c3-gpio-deep-sleep-wake-v1.md`.
//!
//! # GPIO isolation
//!
//! [`EspSleepManager`] calls `esp_sleep_config_gpio_isolate()` automatically
//! before entering deep sleep when `isolate_gpio` is `true` (the default).
//! This prevents floating digital GPIO pins from leaking current during sleep —
//! one of the most commonly missed power optimisations on real boards.
//!
//! A [`WakeSource::GpioLevel`] source combined with `isolate_gpio: true` is
//! refused with `SleepError::IsolationConflict` on every chip, so set
//! `isolate_gpio: false` when using GPIO wake sources.
//! On EXT1 chips isolation may prevent the wake pin from triggering; on ESP32-C2/C3
//! the two may coexist, but that is unverified on hardware, so the refusal stays
//! until bench case I4 decides.
//!
//! # Rejection-aware sleep
//!
//! [`SleepManager::try_sleep`] enters deep sleep via `esp_deep_sleep_try_to_start()`
//! and returns [`EspSleepError::Idf`] with `kind() == SleepErrorKind::Rejected` when a
//! wake source is already asserted, for example a PIR wake pin held at its wake level.
//! The ESP32-C2/C3 hardware does not reject a held GPIO wake pin; it sleeps and wakes
//! again at once. There `try_sleep` reads the wake pins right before sleep entry and
//! returns the same `Rejected` error itself; the pins must be configured as inputs for
//! the read. EXT1 chips rely on the hardware reject (not yet confirmed on hardware).
//! [`SleepManager::sleep`] keeps `esp_deep_sleep_start()`, which sleeps and wakes again
//! at once in that case.
//!
//! # GPIO wake sources and pull resistors
//!
//! When using [`WakeSource::GpioLevel`], an external pull resistor (10–100 kΩ)
//! **against the wake level** is recommended on every wake pin on all chips:
//! a pull-down for `GpioWakeLevel::AnyHigh`, a pull-up for `GpioWakeLevel::AnyLow`.
//! Pins left floating may false-trigger or fail to trigger.
//!
//! - ESP32, ESP32-S3, ESP32-C6 (EXT1): the RTC internal pulls are unavailable in deep
//!   sleep by default, so the external resistor is **required**.
//! - ESP32-C2, ESP32-C3: ESP-IDF enables the internal pull against the wake level at
//!   sleep entry by default (`CONFIG_ESP_SLEEP_GPIO_ENABLE_INTERNAL_RESISTORS`).
//!   Prefer an external resistor anyway: the internal pull is weak and a Kconfig choice.
//!
//! # ESP32-C2/C3: several sleep calls in one boot
//!
//! Each call disarms the GPIO wake pins that an earlier call armed and the new request
//! does not use, also for timer-only requests; pins that were never armed are not touched.
//! Two ESP-IDF v5.3.3 limitations remain within one boot:
//! - A pin armed earlier is still held and pulled during later GPIO-wake sleeps,
//!   although it can no longer wake the chip.
//! - A pin switched from `AnyHigh` to `AnyLow` keeps its internal pull-down during
//!   sleep, which fights a LOW wake (bench case I6).
//!
//! Every deep-sleep wake is a reset, which clears both.
//!
//! # Persisting state across deep sleep
//!
//! Deep sleep clears all RAM except the RTC memory.
//! Place a [`RetainedCell`][crate::RetainedCell] in `.rtc_noinit` so the record also survives
//! watchdog and software resets, not only a sleep/wake cycle.
//! The section is `NOLOAD`: the initialiser is never applied, and the cell
//! validates the record on every `load` instead.
//! Read the reset reason first; `load` refuses the record after a power-on or
//! brownout, when the memory holds arbitrary or partial data.
//! The cell serialises its callers in a critical section, so it may be shared by tasks and interrupts.
//!
//! ```rust,ignore
//! use rustyfarian_esp_idf_power::{EspResetReasonSource, ResetReasonSource, RetainedCell};
//!
//! #[link_section = ".rtc_noinit"]
//! static BOOTS: RetainedCell<4> = RetainedCell::new();
//!
//! fn main() {
//!     let reset = EspResetReasonSource.last_reset_reason();
//!     let boots = BOOTS.load::<u32>(reset).unwrap_or(0).wrapping_add(1);
//!     BOOTS.store(&boots).expect("4 bytes fit in RetainedCell<4>");
//! }
//! ```
//!
//! A complete example is `examples/idf_c3_rtc_retention.rs`.
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

#[cfg(not(all(
    esp_idf_soc_gpio_support_deepsleep_wakeup,
    not(esp_idf_soc_pm_support_ext1_wakeup)
)))]
use stoker::sleep::validate_gpio_level_source;
#[cfg(all(
    esp_idf_soc_gpio_support_deepsleep_wakeup,
    not(esp_idf_soc_pm_support_ext1_wakeup)
))]
use stoker::sleep::{validate_gpio_level_source_against, ESP32C3_WAKE_PIN_MASK};
use stoker::sleep::{
    validate_wake_sources, Error, GpioWakeLevel, GpioWakeMask, SleepError, SleepErrorKind,
    SleepManager, WakeCause, WakeCauseSource, WakeSource, MAX_TIMER_MS,
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
    /// Must be `false` when a [`WakeSource::GpioLevel`] source is used, on every
    /// chip; see the module docs for the per-chip reasons (ESP32-C2/C3 pending
    /// bench case I4).
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
                validate_gpio_wake_mask(*pin_mask)?;
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

        // The disable-all above does not undo per-pin deep-sleep GPIO arming on
        // C2/C3; disarm every wake pin the new request does not use.
        #[cfg(all(
            esp_idf_soc_gpio_support_deepsleep_wakeup,
            not(esp_idf_soc_pm_support_ext1_wakeup)
        ))]
        disarm_unused_gpio_wake_pins(sources)?;

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
                #[cfg(all(
                    esp_idf_soc_gpio_support_deepsleep_wakeup,
                    not(esp_idf_soc_pm_support_ext1_wakeup)
                ))]
                WakeSource::GpioLevel { pin_mask, level } => {
                    let mode = match level {
                        GpioWakeLevel::AnyHigh => {
                            sys::esp_deepsleep_gpio_wake_up_mode_t_ESP_GPIO_WAKEUP_GPIO_HIGH
                        }
                        GpioWakeLevel::AnyLow => {
                            sys::esp_deepsleep_gpio_wake_up_mode_t_ESP_GPIO_WAKEUP_GPIO_LOW
                        }
                    };
                    // SAFETY: esp_deep_sleep_enable_gpio_wakeup() arms the per-pin
                    // deep-sleep GPIO wake enable and ORs the pins into the SDK's wake
                    // mask (sleep_modes.c:1980-2011). pin_mask has been validated
                    // against GPIO0-5 and is non-zero. Must be called after disable-all
                    // and before deep sleep entry.
                    let ret = unsafe { sys::esp_deep_sleep_enable_gpio_wakeup(*pin_mask, mode) };
                    check(ret, "configuring GPIO deep-sleep wakeup")?;
                    log::info!(
                        "Sleep: GPIO deep-sleep wake configured (mask: 0x{:x}, level: {:?})",
                        pin_mask,
                        level
                    );
                }
                #[cfg(not(all(
                    esp_idf_soc_gpio_support_deepsleep_wakeup,
                    not(esp_idf_soc_pm_support_ext1_wakeup)
                )))]
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

/// Validates a `GpioLevel` pin mask against the chip's deep-sleep wake pins.
///
/// C2/C3 use the GPIO0-5 constant explicitly: stoker's `build.rs` sets no cfg for
/// riscv targets, so `validate_gpio_level_source` would pick the S3 mask there.
#[cfg(all(
    esp_idf_soc_gpio_support_deepsleep_wakeup,
    not(esp_idf_soc_pm_support_ext1_wakeup)
))]
fn validate_gpio_wake_mask(pin_mask: u64) -> Result<(), SleepError> {
    validate_gpio_level_source_against(pin_mask, ESP32C3_WAKE_PIN_MASK)
}

/// Validates a `GpioLevel` pin mask against the EXT1 wake pins of the chip.
///
/// The valid mask is chip-specific (ESP32 vs ESP32-S3, selected by the `esp32` cfg);
/// see `validate_gpio_level_source`.
#[cfg(not(all(
    esp_idf_soc_gpio_support_deepsleep_wakeup,
    not(esp_idf_soc_pm_support_ext1_wakeup)
)))]
fn validate_gpio_wake_mask(pin_mask: u64) -> Result<(), SleepError> {
    validate_gpio_level_source(pin_mask)
}

/// Disarms every C2/C3 deep-sleep wake pin that the request does not use.
///
/// `esp_sleep_disable_wakeup_source(ALL)` does not clear per-pin arming, so a pin
/// armed by an earlier sleep would keep its hardware wake enable. Pins in the
/// request's `GpioLevel` mask are left for `esp_deep_sleep_enable_gpio_wakeup`.
/// Pins whose hardware wake enable is clear are not touched, so their sleep-select
/// setting survives.
#[cfg(all(
    esp_idf_soc_gpio_support_deepsleep_wakeup,
    not(esp_idf_soc_pm_support_ext1_wakeup)
))]
fn disarm_unused_gpio_wake_pins(sources: &[WakeSource]) -> Result<(), EspSleepError> {
    let keep = sources
        .iter()
        .find_map(|source| match source {
            WakeSource::GpioLevel { pin_mask, .. } => Some(*pin_mask),
            WakeSource::Timer { .. } => None,
        })
        .unwrap_or(0);
    // ESP-IDF v5.3.3 has no public "is this pin armed for deep-sleep wake" call:
    // the driver offers only enable/disable, and disable on an unarmed pin is not
    // harmless (it re-enables sleep-select, overriding gpio_sleep_sel_dis()). The
    // only test is the inline HAL function below, so read the register it reads.
    // Bench case I7 checks that unrelated pins keep their sleep config.
    //
    // SAFETY: RTC_CNTL_GPIO_WAKEUP_REG is the chip's (C2: base + 0xFC, C3: base + 0x110)
    // always-mapped RTC_CNTL register holding the per-pin deep-sleep wake enables; a
    // volatile 32-bit read has no side effects. Same test as the inline
    // gpio_ll_deepsleep_wakeup_is_enabled (hal/esp32c3/include/hal/gpio_ll.h:751-756),
    // which bindgen cannot expose.
    let wake_reg = unsafe { core::ptr::read_volatile(sys::RTC_CNTL_GPIO_WAKEUP_REG as *const u32) };
    for pin in 0..6u32 {
        let armed = wake_reg & (1 << (sys::RTC_CNTL_GPIO_PIN0_WAKEUP_ENABLE_S - pin)) != 0;
        if !armed || keep & (1u64 << pin) != 0 {
            continue;
        }
        // SAFETY: gpio_deep_sleep_wakeup_disable() clears the pin's hardware wake
        // enable and restores its sleep-select (gpio.c:1001-1017); called only on a
        // pin that is armed for wake. Pins 0-5 are valid GPIO numbers on C2/C3.
        let ret = unsafe { sys::gpio_deep_sleep_wakeup_disable(pin as sys::gpio_num_t) };
        check(ret, "disarming unused GPIO wake pin")?;
    }
    Ok(())
}

/// IO_MUX register of each C2/C3 deep-sleep wake pin, GPIO0-5 (same offsets on both chips).
#[cfg(all(
    esp_idf_soc_gpio_support_deepsleep_wakeup,
    not(esp_idf_soc_pm_support_ext1_wakeup)
))]
const WAKE_PIN_IO_MUX_REGS: [u32; 6] = [
    sys::PERIPHS_IO_MUX_XTAL_32K_P_U,
    sys::PERIPHS_IO_MUX_XTAL_32K_N_U,
    sys::PERIPHS_IO_MUX_GPIO2_U,
    sys::PERIPHS_IO_MUX_GPIO3_U,
    sys::PERIPHS_IO_MUX_MTMS_U,
    sys::PERIPHS_IO_MUX_MTDI_U,
];

/// Returns `true` when a C2/C3 `GpioLevel` wake pin is already at its wake level.
///
/// The C2/C3 hardware does not reject deep sleep for such a pin: the chip sleeps and
/// wakes again at once (bench H1), so [`try_sleep`][SleepManager::try_sleep] reads the
/// pins itself. Only pins with the pad input enable set are read, because
/// `gpio_get_level` returns 0 for a pad without it; a pin without input enable is
/// skipped with a warning.
#[cfg(all(
    esp_idf_soc_gpio_support_deepsleep_wakeup,
    not(esp_idf_soc_pm_support_ext1_wakeup)
))]
fn gpio_wake_pins_asserted(sources: &[WakeSource]) -> bool {
    // validate_wake_sources allows at most one GpioLevel source.
    let Some((pin_mask, level)) = sources.iter().find_map(|source| match source {
        WakeSource::GpioLevel { pin_mask, level } => Some((*pin_mask, *level)),
        WakeSource::Timer { .. } => None,
    }) else {
        return false;
    };
    let wake_level = match level {
        GpioWakeLevel::AnyHigh => 1,
        GpioWakeLevel::AnyLow => 0,
    };
    for (pin, &mux_reg) in WAKE_PIN_IO_MUX_REGS.iter().enumerate() {
        if pin_mask & (1u64 << pin) == 0 {
            continue;
        }
        // SAFETY: mux_reg is the always-mapped IO_MUX register of GPIO0-5 on C2/C3
        // (io_mux_reg.h); a volatile 32-bit read has no side effects. The input enable
        // is bit FUN_IE_S, the bit gpio_ll_input_enable sets.
        let mux = unsafe { core::ptr::read_volatile(mux_reg as *const u32) };
        if mux & (1 << sys::FUN_IE_S) == 0 {
            log::warn!(
                "Sleep: GPIO{} has no input enable; its level is not checked before sleep",
                pin
            );
            continue;
        }
        // SAFETY: gpio_get_level only reads GPIO_IN_REG; pin is a valid GPIO number (0-5).
        let read = unsafe { sys::gpio_get_level(pin as sys::gpio_num_t) };
        if read == wake_level {
            log::info!("Sleep: GPIO{} is already at its wake level", pin);
            return true;
        }
    }
    false
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
    ///   On ESP32-C2/C3 the wake pins also stay held (`gpio_hold_en`) with the
    ///   internal pull applied at sleep entry; call `gpio_hold_dis` before
    ///   reconfiguring such a pin for other use.
    ///   On ESP32-C2/C3 the hardware does not reject a held GPIO wake pin, so the
    ///   manager reads the `GpioLevel` pins right before sleep entry and returns this
    ///   error itself (same `code`); the wake sources stay configured and no pin is held.
    ///   Only pins configured as inputs are read, and a pin needs an external bias
    ///   against the wake level, because the internal pull is applied after the read.
    ///   A pin that reaches its wake level after the read still wakes the chip at once.
    fn try_sleep(&mut self, sources: &[WakeSource]) -> Result<(), EspSleepError> {
        self.configure(sources)?;

        // The C2/C3 hardware does not reject a held GPIO wake pin; it sleeps and wakes
        // at once. Read the pins as late as possible, right before sleep entry.
        #[cfg(all(
            esp_idf_soc_gpio_support_deepsleep_wakeup,
            not(esp_idf_soc_pm_support_ext1_wakeup)
        ))]
        if gpio_wake_pins_asserted(sources) {
            // Same shape as the hardware reject below: ESP_ERR_SLEEP_REJECT is
            // ESP_ERR_INVALID_STATE in IDF v5.3.3, and bindgen exposes only the latter.
            return Err(EspSleepError::Idf {
                code: sys::ESP_ERR_INVALID_STATE,
                kind: SleepErrorKind::Rejected,
            });
        }

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
            // ESP32-C2/C3 have no EXT1 and no `esp_sleep_get_ext1_wakeup_status`
            // symbol; there the cause falls through to `Other`.
            #[cfg(esp_idf_soc_pm_support_ext1_wakeup)]
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
                // ESP32-C2/C3/C6 feature (SOC_GPIO_SUPPORT_DEEPSLEEP_WAKEUP), not S3.
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
/// (ESP32-C2/C3/C6, esp-idf-sys cfg `esp_idf_soc_gpio_support_deepsleep_wakeup`).
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
