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
//! | ESP32, ESP32-S3, ESP32-C6 | EXT1 (`esp_sleep_enable_ext1_wakeup_io`, RTC IO) | chip specific, see `ChipPolicy::wake_pin_mask` |
//! | ESP32-C2, ESP32-C3 | deep-sleep GPIO wakeup (`esp_deep_sleep_enable_gpio_wakeup`) | GPIO0-5 (`ESP32C3_WAKE_PIN_MASK`) |
//!
//! The pin mask is validated before any ESP-IDF call.
//! Implementation notes against ESP-IDF v5.3.3 are in
//! `docs/features/archive/c3-gpio-deep-sleep-wake-v1.md`.
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
//! the two may coexist, but that is unverified on hardware, and the refusal stays
//! by decision; bench case I4 can relax it if a consumer needs the current saving.
//!
//! # Rejection-aware sleep
//!
//! [`SleepManager::try_sleep`] enters deep sleep via `esp_deep_sleep_try_to_start()`
//! and returns [`EspSleepError::Idf`] with `kind() == SleepErrorKind::Rejected` when a
//! wake source is already asserted, for example a PIR wake pin held at its wake level.
//! The ESP32-C2/C3 hardware does not reject a held GPIO wake pin; it sleeps and wakes
//! again at once. There `try_sleep` reads the wake pins right before sleep entry and
//! returns the same `Rejected` error itself; the pins must be configured as inputs for
//! the read. The original ESP32 reads the wake pins on every attempt too, and the ESP32-S3
//! when the global digital-pad hold is effective (see [`try_sleep`][SleepManager::try_sleep]).
//! Other EXT1 chips rely on the hardware reject (confirmed on the ESP32-S3,
//! CrowPanel bench 2026-10-14).
//! The sequence is the shared `stoker::SleepAttemptMachine`; this tier only does the IO.
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
//! A rejected [`try_sleep`][SleepManager::try_sleep] releases the holds and wake
//! enables of its own attempt before returning, so only pins armed by *earlier*
//! attempts can remain held within a boot.
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
//! The cell serialises its callers in a critical section, so it may be shared by tasks and interrupts
//! when the firmware links an interrupt-safe provider such as the `critical-section-impl` feature.
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

#[cfg(any(esp32, esp32s3))]
use stoker::sleep::digital_hold_effective;
use stoker::sleep::{
    Error, GpioWakeLevel, GpioWakeMask, SleepError, SleepErrorKind, SleepManager, WakeCause,
    WakeCauseSource, WakeSource, MAX_TIMER_MS,
};
use stoker::{
    arm_transactionally, drive, ChipPolicy, DriveError, Entered, SleepAttemptMachine,
    SleepAttemptPort, SleepPlan, WakeArmOps,
};

/// EXT1 configuration armed before the last deep sleep, kept in RTC slow memory (ESP32 only).
///
/// ESP-IDF's boot code clears the EXT1 pin selection before `app_main`
/// (`cpu_start.c` → `esp_deep_sleep_wakeup_io_reset`), and in the ESP32's low-level (`ALL_LOW`)
/// mode `esp_sleep_get_ext1_wakeup_status()` then reads an empty mask (bench 2026-11-09).
/// [`EspWakeCauseSource`] falls back to this record through [`stoker::ext1_fired_mask`].
/// `.rtc_noinit` stays powered in deep sleep because IDF's AUTO rule keeps RTC slow memory on
/// while it holds data; the initialiser never runs there, so the words are garbage until the
/// check word validates them. Atomics give interior mutability, so the compiler cannot assume
/// their initial value; one writer before sleep, one reader at boot, no read-modify-write.
#[cfg(esp32)]
mod armed_ext1 {
    use core::sync::atomic::{AtomicU32, Ordering};

    /// Mixed into the check word, so zeroed or random memory does not validate.
    const MAGIC: u32 = 0x4558_3141;

    /// Mask low word, mask high word, low-level flag, check word.
    ///
    /// ESP-IDF's ESP32 linker script places `.rtc_noinit` as NOLOAD in RTC slow memory
    /// (`esp_system/ld/esp32/sections.ld.in`); without that section the record would be lost.
    #[link_section = ".rtc_noinit"]
    static RECORD: [AtomicU32; 4] = [const { AtomicU32::new(0) }; 4];

    /// This boot's answer, in normal RAM (zero at boot): taken flag, mask low, mask high,
    /// bit 1 = record valid, bit 0 = low level.
    static TAKEN: [AtomicU32; 4] = [const { AtomicU32::new(0) }; 4];

    fn check(lo: u32, hi: u32, low: u32) -> u32 {
        MAGIC ^ lo ^ hi.rotate_left(8) ^ low.rotate_left(16)
    }

    /// Records the armed EXT1 mask and whether it wakes on a low level; the check word last.
    pub(super) fn store(mask: u64, low: bool) {
        // The mask is split into its low and high words on purpose.
        #[allow(clippy::cast_possible_truncation)]
        let (lo, hi) = (mask as u32, (mask >> 32) as u32);
        let low_flag = u32::from(low);
        RECORD[0].store(lo, Ordering::Relaxed);
        RECORD[1].store(hi, Ordering::Relaxed);
        RECORD[2].store(low_flag, Ordering::Relaxed);
        RECORD[3].store(check(lo, hi, low_flag), Ordering::Release);
    }

    /// Invalidates the record: all zero, and `check(0, 0, 0)` is `MAGIC`, not zero.
    pub(super) fn invalidate() {
        for word in &RECORD {
            word.store(0, Ordering::Relaxed);
        }
    }

    /// The armed mask and low-level flag, if the record validates.
    fn load() -> Option<(u64, bool)> {
        let stored = RECORD[3].load(Ordering::Acquire);
        let lo = RECORD[0].load(Ordering::Relaxed);
        let hi = RECORD[1].load(Ordering::Relaxed);
        let low = RECORD[2].load(Ordering::Relaxed);
        (stored == check(lo, hi, low) && low <= 1)
            .then(|| (u64::from(lo) | (u64::from(hi) << 32), low == 1))
    }

    /// The record of the sleep that ended this boot: the first call loads it and invalidates it
    /// in RTC memory, so a later sleep armed outside [`EspSleepManager`](super::EspSleepManager)
    /// cannot inherit it; later calls in the same boot return the same answer.
    pub(super) fn take() -> Option<(u64, bool)> {
        if TAKEN[0].load(Ordering::Acquire) == 0 {
            let flags = match load() {
                Some((mask, low)) => {
                    #[allow(clippy::cast_possible_truncation)]
                    let (lo, hi) = (mask as u32, (mask >> 32) as u32);
                    TAKEN[1].store(lo, Ordering::Relaxed);
                    TAKEN[2].store(hi, Ordering::Relaxed);
                    0b10 | u32::from(low)
                }
                None => 0,
            };
            TAKEN[3].store(flags, Ordering::Relaxed);
            invalidate();
            TAKEN[0].store(1, Ordering::Release);
        }
        let flags = TAKEN[3].load(Ordering::Relaxed);
        (flags & 0b10 != 0).then(|| {
            let mask = u64::from(TAKEN[1].load(Ordering::Relaxed))
                | (u64::from(TAKEN[2].load(Ordering::Relaxed)) << 32);
            (mask, flags & 1 != 0)
        })
    }
}

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
    /// chip; see the module docs for the per-chip reasons.
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
    /// Validates, then applies the wake configuration of `plan` through
    /// [`arm_transactionally`].
    ///
    /// The plan comes from [`SleepAttemptMachine::new`], which validated the whole request;
    /// the isolation check here is the last validation and touches no hardware.
    /// A failing SDK call after that starts the shared rollback, which attempts to disarm
    /// every wake source (the `SleepAttemptPort::arm` contract); the SDK error is returned.
    fn configure(&self, plan: SleepPlan) -> Result<(), EspSleepError> {
        if plan.has_pins() && self.isolate_gpio {
            return Err(SleepError::IsolationConflict.into());
        }
        arm_transactionally(&mut IdfWakeArmOps { manager: self }, plan)
    }
}

/// ESP-IDF SDK calls of one transactional arm; order and rollback are `stoker`'s.
struct IdfWakeArmOps<'a> {
    manager: &'a EspSleepManager,
}

impl WakeArmOps for IdfWakeArmOps<'_> {
    type Error = EspSleepError;

    fn clear(&mut self, keep_pins: u64) -> Result<(), EspSleepError> {
        // Every arm starts here, timer-only included, so no later wake reads an old record.
        #[cfg(esp32)]
        armed_ext1::invalidate();

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

        // The disable-all above does not clear s_config.ext1_rtc_gpio_mask or the
        // trigger mode (sleep_modes.c:1544-1545 skips the per-source clear at
        // 1555-1559), and esp_sleep_enable_ext1_wakeup_io OR-merges, so a retry
        // with a changed EXT1 pin/level combination would fail the mask merge with
        // ESP_ERR_NOT_ALLOWED (sleep_modes.c:1791-1810). Clear it unconditionally.
        #[cfg(not(all(
            esp_idf_soc_gpio_support_deepsleep_wakeup,
            not(esp_idf_soc_pm_support_ext1_wakeup)
        )))]
        {
            let _ = keep_pins;
            clear_ext1_state("clearing stale EXT1 wake state")?;
        }

        // The disable-all above does not undo per-pin deep-sleep GPIO arming on
        // C2/C3; disarm every wake pin the new request does not use.
        #[cfg(all(
            esp_idf_soc_gpio_support_deepsleep_wakeup,
            not(esp_idf_soc_pm_support_ext1_wakeup)
        ))]
        disarm_unused_gpio_wake_pins(keep_pins)?;

        Ok(())
    }

    fn arm_timer(&mut self, duration_ms: u64) -> Result<(), EspSleepError> {
        // Unreachable after SleepAttemptMachine::new caps the duration at
        // MAX_TIMER_MS; the arithmetic stays checked.
        let duration_us = duration_ms
            .checked_mul(1_000)
            .ok_or(SleepError::TimerTooLong {
                duration_ms,
                max_ms: MAX_TIMER_MS,
            })?;
        // SAFETY: esp_sleep_enable_timer_wakeup configures the RTC
        // timer wakeup source. Safe to call before deep sleep entry;
        // duration_us has been validated above.
        let ret = unsafe { sys::esp_sleep_enable_timer_wakeup(duration_us) };
        check(ret, "configuring timer wakeup")?;
        log::info!("Sleep: timer wake configured for {}ms", duration_ms);
        Ok(())
    }

    #[cfg(all(
        esp_idf_soc_gpio_support_deepsleep_wakeup,
        not(esp_idf_soc_pm_support_ext1_wakeup)
    ))]
    fn arm_pins(&mut self, pin_mask: u64, level: GpioWakeLevel) -> Result<(), EspSleepError> {
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
        let ret = unsafe { sys::esp_deep_sleep_enable_gpio_wakeup(pin_mask, mode) };
        check(ret, "configuring GPIO deep-sleep wakeup")?;
        log::info!(
            "Sleep: GPIO deep-sleep wake configured (mask: 0x{:x}, level: {:?})",
            pin_mask,
            level
        );
        Ok(())
    }

    #[cfg(not(all(
        esp_idf_soc_gpio_support_deepsleep_wakeup,
        not(esp_idf_soc_pm_support_ext1_wakeup)
    )))]
    fn arm_pins(&mut self, pin_mask: u64, level: GpioWakeLevel) -> Result<(), EspSleepError> {
        let mode = match level {
            GpioWakeLevel::AnyHigh => sys::esp_sleep_ext1_wakeup_mode_t_ESP_EXT1_WAKEUP_ANY_HIGH,
            GpioWakeLevel::AnyLow => {
                // ESP_EXT1_WAKEUP_ANY_LOW is only available on ESP32-S3 and
                // newer chips. The original ESP32 only exposes ALL_LOW, which is
                // equivalent to any-low for the single pin that SleepAttemptMachine::new
                // lets through there (multi-pin AnyLow is rejected on that chip).
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
        let ret = unsafe { sys::esp_sleep_enable_ext1_wakeup_io(pin_mask, mode) };
        check(ret, "configuring GPIO EXT1 wakeup")?;
        #[cfg(esp32)]
        armed_ext1::store(pin_mask, level == GpioWakeLevel::AnyLow);
        log::info!(
            "Sleep: EXT1 GPIO wake configured (mask: 0x{:x}, level: {:?})",
            pin_mask,
            level
        );
        Ok(())
    }

    fn finish(&mut self) -> Result<(), EspSleepError> {
        if self.manager.isolate_gpio {
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

    fn undo_sources(&mut self) -> Result<(), EspSleepError> {
        // A rolled-back arm must not leave a record behind.
        #[cfg(esp32)]
        armed_ext1::invalidate();

        // SAFETY: as in `clear`; disabling every wake source is safe at any time
        // before deep sleep entry.
        let ret = unsafe {
            sys::esp_sleep_disable_wakeup_source(sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_ALL)
        };
        let all = check(ret, "rolling back wake sources");
        #[cfg(not(all(
            esp_idf_soc_gpio_support_deepsleep_wakeup,
            not(esp_idf_soc_pm_support_ext1_wakeup)
        )))]
        let all = all.and(clear_ext1_state("rolling back EXT1 wake state"));
        all
    }

    fn armed_pins(&mut self) -> u64 {
        #[cfg(all(
            esp_idf_soc_gpio_support_deepsleep_wakeup,
            not(esp_idf_soc_pm_support_ext1_wakeup)
        ))]
        {
            armed_gpio_wake_pins()
        }
        // EXT1 chips have no per-pin enable to undo; `undo_sources` cleared EXT1.
        #[cfg(not(all(
            esp_idf_soc_gpio_support_deepsleep_wakeup,
            not(esp_idf_soc_pm_support_ext1_wakeup)
        )))]
        {
            0
        }
    }

    fn undo_pin(&mut self, pin: u8) -> Result<(), EspSleepError> {
        #[cfg(all(
            esp_idf_soc_gpio_support_deepsleep_wakeup,
            not(esp_idf_soc_pm_support_ext1_wakeup)
        ))]
        {
            disarm_gpio_wake_pin(u32::from(pin), "rolling back GPIO wake pin")
        }
        #[cfg(not(all(
            esp_idf_soc_gpio_support_deepsleep_wakeup,
            not(esp_idf_soc_pm_support_ext1_wakeup)
        )))]
        {
            let _ = pin;
            Ok(())
        }
    }
}

/// Clears the SDK's EXT1 pin mask and trigger mode.
#[cfg(not(all(
    esp_idf_soc_gpio_support_deepsleep_wakeup,
    not(esp_idf_soc_pm_support_ext1_wakeup)
)))]
fn clear_ext1_state(what: &str) -> Result<(), EspSleepError> {
    // SAFETY: esp_sleep_disable_ext1_wakeup_io with a zero mask writes only
    // the SDK's static s_config fields (mask and trigger mode) and clears
    // the RTC_EXT1_TRIG_EN bit the disable-all already cleared
    // (sleep_modes.c:1817-1843); safe to call before deep sleep entry.
    let ret = unsafe { sys::esp_sleep_disable_ext1_wakeup_io(0) };
    check(ret, what)
}

/// Mask of the C2/C3 pins whose deep-sleep wake enable bit is set in
/// `RTC_CNTL_GPIO_WAKEUP_REG` (bit N = GPIO N, N in 0..6).
///
/// ESP-IDF v5.3.3 has no public "is this pin armed for deep-sleep wake" call:
/// the driver offers only enable/disable, and disable on an unarmed pin is not
/// harmless (it re-enables sleep-select, overriding gpio_sleep_sel_dis()). The
/// only test is the inline HAL function below, so read the register it reads.
/// Shared by `disarm_unused_gpio_wake_pins` and the hold release in
/// [`try_sleep`][SleepManager::try_sleep], so the subtle reversed-bit test lives
/// in exactly one place.
///
/// SAFETY (in the body): RTC_CNTL_GPIO_WAKEUP_REG is the chip's (C2: base + 0xFC,
/// C3: base + 0x110) always-mapped RTC_CNTL register holding the per-pin
/// deep-sleep wake enables; a volatile 32-bit read has no side effects. Same
/// test as the inline gpio_ll_deepsleep_wakeup_is_enabled
/// (hal/esp32c3/include/hal/gpio_ll.h:751-756), which bindgen cannot expose.
#[cfg(all(
    esp_idf_soc_gpio_support_deepsleep_wakeup,
    not(esp_idf_soc_pm_support_ext1_wakeup)
))]
fn armed_gpio_wake_pins() -> u64 {
    let wake_reg = unsafe { core::ptr::read_volatile(sys::RTC_CNTL_GPIO_WAKEUP_REG as *const u32) };
    (0..6u32)
        .filter(|pin| wake_reg & (1 << (sys::RTC_CNTL_GPIO_PIN0_WAKEUP_ENABLE_S - pin)) != 0)
        .fold(0u64, |mask, pin| mask | (1u64 << pin))
}

/// Disarms every C2/C3 deep-sleep wake pin that the request does not use.
///
/// `esp_sleep_disable_wakeup_source(ALL)` does not clear per-pin arming, so a pin
/// armed by an earlier sleep would keep its hardware wake enable. Pins in the
/// request's `GpioLevel` mask are left for `esp_deep_sleep_enable_gpio_wakeup`.
/// Pins whose hardware wake enable is clear are not touched, so their sleep-select
/// setting survives.
/// Bench case I7 checks that unrelated pins keep their sleep config.
#[cfg(all(
    esp_idf_soc_gpio_support_deepsleep_wakeup,
    not(esp_idf_soc_pm_support_ext1_wakeup)
))]
fn disarm_unused_gpio_wake_pins(keep: u64) -> Result<(), EspSleepError> {
    let disarm = armed_gpio_wake_pins() & !keep;
    for pin in 0..6u32 {
        if disarm & (1u64 << pin) == 0 {
            continue;
        }
        disarm_gpio_wake_pin(pin, "disarming unused GPIO wake pin")?;
    }
    Ok(())
}

/// Clears one C2/C3 pin's deep-sleep wake enable.
#[cfg(all(
    esp_idf_soc_gpio_support_deepsleep_wakeup,
    not(esp_idf_soc_pm_support_ext1_wakeup)
))]
fn disarm_gpio_wake_pin(pin: u32, what: &str) -> Result<(), EspSleepError> {
    // SAFETY: gpio_deep_sleep_wakeup_disable() clears the pin's hardware wake
    // enable and restores its sleep-select (gpio.c:1001-1017); called only on a
    // pin that is armed for wake. Pins 0-5 are valid GPIO numbers on C2/C3.
    let ret = unsafe { sys::gpio_deep_sleep_wakeup_disable(pin as sys::gpio_num_t) };
    check(ret, what)
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

/// Chip policy of this build for [`SleepAttemptMachine`].
///
/// ESP32-C2 shares the C3 row (same guard); the ESP32-C6 has its own row, which never
/// reads the pins in software. Other chips map to [`ChipPolicy::Other`].
const fn chip_policy() -> ChipPolicy {
    if cfg!(esp32) {
        ChipPolicy::Esp32
    } else if cfg!(esp32s3) {
        ChipPolicy::Esp32S3
    } else if cfg!(esp32c6) {
        ChipPolicy::Esp32C6
    } else if cfg!(all(
        esp_idf_soc_gpio_support_deepsleep_wakeup,
        not(esp_idf_soc_pm_support_ext1_wakeup)
    )) {
        ChipPolicy::Esp32C3
    } else {
        ChipPolicy::Other
    }
}

/// IO_MUX register of a wake pin; the pin was validated against the chip's wake pins.
#[cfg(all(
    esp_idf_soc_gpio_support_deepsleep_wakeup,
    not(esp_idf_soc_pm_support_ext1_wakeup)
))]
fn wake_pin_mux_reg(pin: u32) -> u32 {
    WAKE_PIN_IO_MUX_REGS[pin as usize]
}

/// IO_MUX register of a wake pin; the pin was validated against the chip's RTC wake pins.
#[cfg(not(all(
    esp_idf_soc_gpio_support_deepsleep_wakeup,
    not(esp_idf_soc_pm_support_ext1_wakeup)
)))]
fn wake_pin_mux_reg(pin: u32) -> u32 {
    // SAFETY: the mask was validated against the chip's RTC wake pins, all below
    // SOC_GPIO_PIN_COUNT, so the index is in range of the register table.
    unsafe { sys::GPIO_PIN_MUX_REG[pin as usize] }
}

/// Reads the `GpioLevel` wake pins for [`SleepAttemptMachine`]; bit N set = GPIO N high.
///
/// Only pins with the pad input enable set are read, because `gpio_get_level` returns 0
/// for a pad without it; a pin without input enable is skipped with a warning and
/// reported at the level opposite to its wake level, which keeps it from tripping the
/// guard (the guard is void for that pin).
/// The read happens after the wake sources are configured but before sleep entry runs
/// the prepare hooks that RTC-route the pins, so it sees the digital level.
/// For the same reason the internal pull that sleep entry applies is not active yet: a
/// wake pin needs an external bias (or a pull set on its digital driver) against its
/// wake level, or a floating pin can read as asserted and every retry is rejected while
/// the hardware would have slept.
fn read_wake_pins(mask: u64, level: GpioWakeLevel) -> u64 {
    let mut levels = 0u64;
    for pin in 0..64u32 {
        if mask & (1u64 << pin) == 0 {
            continue;
        }
        // SAFETY: each IO_MUX register of a wake pin is always mapped; a volatile 32-bit
        // read has no side effects. FUN_IE_S is the bit gpio_ll_input_enable sets.
        let mux = unsafe { core::ptr::read_volatile(wake_pin_mux_reg(pin) as *const u32) };
        if mux & (1 << sys::FUN_IE_S) == 0 {
            log::warn!(
                "Sleep: GPIO{} has no input enable; its level is not checked before sleep",
                pin
            );
            if level == GpioWakeLevel::AnyLow {
                levels |= 1u64 << pin;
            }
            continue;
        }
        // SAFETY: gpio_get_level only reads the GPIO input register; pin is a valid GPIO.
        if unsafe { sys::gpio_get_level(pin as sys::gpio_num_t) } != 0 {
            levels |= 1u64 << pin;
        }
    }
    levels
}

/// Reads whether the global digital-pad hold is effective (ESP32 and ESP32-S3).
///
/// With the hold effective (`RTC_CNTL_DIG_ISO_REG` bit 11 set, bit 14 clear), sleep entry
/// isolates every unheld digital pad and `esp_deep_sleep_try_to_start()` never returns
/// when the hardware rejects the sleep (interrupt watchdog reset, bench R2-S3).
#[cfg(any(esp32, esp32s3))]
fn read_hold_effective() -> bool {
    // SAFETY: RTC_CNTL_DIG_ISO_REG is an always-mapped RTC_CNTL register on both chips
    // (ESP32: base + 0x88, ESP32-S3: base + 0x94); a volatile 32-bit read has no side effects.
    let dig_iso = unsafe { core::ptr::read_volatile(sys::RTC_CNTL_DIG_ISO_REG as *const u32) };
    digital_hold_effective(dig_iso)
}

/// No other chip is given the hold read by [`chip_policy`].
#[cfg(not(any(esp32, esp32s3)))]
fn read_hold_effective() -> bool {
    false
}

/// Best-effort restore of the request's wake pins as plain digital inputs after a
/// hardware rejection (ADR-003 point 1).
///
/// The prepare hooks (ext1_wakeup_prepare / gpio_deep_sleep_wakeup_prepare) RTC-route
/// and hold the wake pins before the reject decision (sleep_modes.c:840-850) and the
/// deep-sleep reject tail undoes neither, so release them here: the hold alone blocks
/// reconfiguration, and the RTC routing alone blocks gpio_get_level.
///
/// Deliberate exception to the module's error convention: the release is best-effort.
/// The caller is owed kind Rejected; a release failure must not morph it into
/// Platform, and a leftover hold is the previously shipped behaviour, so a failure is
/// logged and the loop continues.
#[cfg(not(all(
    esp_idf_soc_gpio_support_deepsleep_wakeup,
    not(esp_idf_soc_pm_support_ext1_wakeup)
)))]
fn release_wake_pins(pin_mask: u64) {
    for pin in 0..64u32 {
        if pin_mask & (1u64 << pin) == 0 {
            continue;
        }
        // SAFETY: rtc_gpio_hold_dis releases the RTC pad hold that
        // ext1_wakeup_prepare applied (rtcio_hal_hold_enable,
        // sleep_modes.c:1874-1896); it is idempotent (a pure bit-clear)
        // on an unheld pin. The mask was validated against the chip's
        // RTC wake pins, so every pin is a valid RTC GPIO — including
        // input-only pins 34-39 on the original ESP32, which
        // gpio_hold_dis would refuse (GPIO_IS_VALID_OUTPUT_GPIO), hence
        // the rtc_ variant here.
        let ret = unsafe { sys::rtc_gpio_hold_dis(pin as sys::gpio_num_t) };
        if ret != sys::ESP_OK {
            log::warn!(
                "Sleep: releasing RTC hold on GPIO{} failed: ESP-IDF error 0x{:08x}",
                pin,
                ret
            );
        }
        // SAFETY: ext1_wakeup_prepare also RTC-routed the pad
        // (rtcio_hal_function_select, MUX_SEL at sleep_modes.c:1890), which
        // disconnects it from the digital domain even though the digital
        // IO_MUX config is untouched — gpio_get_level would read garbage.
        // rtc_gpio_deinit restores the digital routing (rtc_io.c:59-72; on
        // S3/C6 it also balances the prepare hook's LP-IO clock enable);
        // it does NOT release the hold, hence the hold_dis first, mirroring
        // the boot-time release order (sleep_gpio.c:156-171). Idempotent on
        // a digitally routed pin; the only failure is EINVAL on a non-RTC
        // GPIO, which the validated mask rules out.
        let ret = unsafe { sys::rtc_gpio_deinit(pin as sys::gpio_num_t) };
        if ret != sys::ESP_OK {
            log::warn!(
                "Sleep: restoring digital routing on GPIO{} failed: ESP-IDF error 0x{:08x}",
                pin,
                ret
            );
        }
    }
}

/// Best-effort restore of the armed C2/C3 wake pins after a hardware rejection
/// (ADR-003 point 1); see the EXT1 variant for the reasoning.
/// The pins to restore are read back from the wake-enable register, not taken from the
/// request, so only pins that are actually armed are touched.
#[cfg(all(
    esp_idf_soc_gpio_support_deepsleep_wakeup,
    not(esp_idf_soc_pm_support_ext1_wakeup)
))]
fn release_wake_pins(_pin_mask: u64) {
    let armed = armed_gpio_wake_pins();
    for pin in 0..6u32 {
        if armed & (1u64 << pin) == 0 {
            continue;
        }
        // SAFETY: gpio_deep_sleep_wakeup_disable clears the pin's hardware
        // wake enable and restores its sleep-select (gpio.c:1001-1017);
        // called only on a pin whose armed bit is set — on an unarmed pin
        // it would re-enable sleep-select. gpio_hold_dis releases the
        // hold gpio_deep_sleep_wakeup_prepare applied (gpio_hold_en,
        // sleep_modes.c:1953-1982); pins 0-5 are output-capable, so
        // gpio_hold_dis is valid here (unlike input-only EXT1 pins).
        let ret = unsafe { sys::gpio_deep_sleep_wakeup_disable(pin as sys::gpio_num_t) };
        if ret != sys::ESP_OK {
            log::warn!(
                "Sleep: disarming GPIO wake pin {} failed: ESP-IDF error 0x{:08x}",
                pin,
                ret
            );
        }
        let ret = unsafe { sys::gpio_hold_dis(pin as sys::gpio_num_t) };
        if ret != sys::ESP_OK {
            log::warn!(
                "Sleep: releasing hold on GPIO{} failed: ESP-IDF error 0x{:08x}",
                pin,
                ret
            );
        }
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
        let plan = SleepAttemptMachine::new(chip_policy(), sources)?.plan();
        self.configure(plan)?;

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
    ///   The prepare hooks RTC-route and hold the wake pins before the hardware
    ///   reject decision, so a hardware rejection restores them before returning:
    ///   on EXT1 chips the RTC routing (`rtc_gpio_deinit`) and the pad hold are
    ///   released per masked pin, on ESP32-C2/C3 the hold and the per-pin wake
    ///   enable are cleared per armed pin. The request's wake pins are therefore
    ///   usable as plain digital inputs right away (ADR-003 point 1) — a pin the
    ///   application was using through the RTC/analog domain before the sleep
    ///   attempt comes back digital and must be re-initialised for that use.
    ///   Only pins armed by *earlier* attempts in the same boot may remain held
    ///   (see the module section on several sleep calls).
    ///   The restore is best-effort and never changes this error's kind.
    ///   On ESP32-C2/C3 the hardware does not reject a held GPIO wake pin, so the
    ///   manager reads the `GpioLevel` pins right before sleep entry and returns this
    ///   error itself (same `code`); the wake sources stay configured and no pin is held.
    ///   On C2/C3 the per-pin wake enable and the hold are cleared again, as after a hardware reject.
    ///   Only pins configured as inputs are read, and a pin needs an external bias
    ///   against the wake level, because the internal pull is applied after the read.
    ///   A pin that reaches its wake level after the read still wakes the chip at once.
    ///   On the original ESP32 the manager reads the `GpioLevel` pins right before sleep
    ///   entry on every attempt, whether or not the hold below is effective, and applies
    ///   the same guard as the hardware (`AnyLow` with several pins is rejected there as
    ///   `Unsupported`; one `AnyLow` pin uses EXT1 `ALL_LOW`, which is equivalent). This closes
    ///   the earlier difference where only the hold case was guarded.
    ///   On ESP32 and ESP32-S3 the hardware never returns from a rejected sleep while the
    ///   global digital-pad hold is effective (`RTC_CNTL_DIG_ISO_REG` bit 11 set, bit 14
    ///   clear, for example after `gpio_deep_sleep_hold_en()`): sleep entry isolates every
    ///   unheld digital pad and the interrupt watchdog resets the chip. With the hold
    ///   effective, the manager therefore reads the `GpioLevel` pins right before sleep
    ///   entry and returns this error itself (same `code`), logging a warning; a pin without
    ///   input enable is skipped with a warning. The global hold costs the isolation of every
    ///   unheld digital pad; per-pin holds on RTC pads do not need it. A pin that reaches its
    ///   wake level after the read, and non-pin hardware rejections, can still hang while the
    ///   hold is effective.
    fn try_sleep(&mut self, sources: &[WakeSource]) -> Result<(), EspSleepError> {
        // The machine decides when each step runs; this tier does the register reads,
        // the sleep call and the pin release.
        let machine = SleepAttemptMachine::new(chip_policy(), sources)?;
        let mut port = IdfSleepPort {
            manager: self,
            level: machine.plan().level,
            returned_code: None,
        };
        match drive(machine, &mut port) {
            Ok(()) => Ok(()),
            Err(DriveError::Arm(err)) => Err(err),
            // Mapped by call site: any return from esp_deep_sleep_try_to_start is a
            // rejection. ESP_ERR_SLEEP_REJECT aliases ESP_ERR_INVALID_STATE, so the code
            // alone cannot identify it; a software reject uses the same code.
            Err(DriveError::Rejected(kind)) => Err(EspSleepError::Idf {
                code: port.returned_code.unwrap_or(sys::ESP_ERR_INVALID_STATE),
                kind,
            }),
            // The port answers every action in phase, so this cannot happen.
            Err(DriveError::Protocol) => Err(EspSleepError::Idf {
                code: sys::ESP_FAIL,
                kind: SleepErrorKind::Platform,
            }),
        }
    }
}

/// IO of one [`try_sleep`][SleepManager::try_sleep] attempt for `stoker::drive`.
struct IdfSleepPort<'a> {
    manager: &'a EspSleepManager,
    level: GpioWakeLevel,
    /// The code of the sleep call that returned, if any; a software reject has none.
    returned_code: Option<sys::esp_err_t>,
}

impl SleepAttemptPort for IdfSleepPort<'_> {
    type Error = EspSleepError;

    fn arm(&mut self, plan: SleepPlan) -> Result<(), EspSleepError> {
        self.manager.configure(plan)
    }

    fn read_hold(&mut self) -> bool {
        read_hold_effective()
    }

    fn read_pins(&mut self, mask: u64) -> u64 {
        read_wake_pins(mask, self.level)
    }

    fn enter(&mut self) -> Entered {
        log::info!("Trying to enter deep sleep");

        // SAFETY: All wake sources have been configured above. GPIO pins are
        // isolated (when isolate_gpio is true). esp_deep_sleep_try_to_start()
        // does not return when the device sleeps; it returns only when the sleep
        // request is rejected.
        let code = unsafe { sys::esp_deep_sleep_try_to_start() };
        if code == sys::ESP_OK {
            Entered::Slept
        } else {
            self.returned_code = Some(code);
            Entered::Returned
        }
    }

    fn release(&mut self, pins: u64) {
        if pins == 0 {
            // Timer-only plan: no pin was armed, held or RTC-routed, and the next attempt
            // disables every wake source first, so there is nothing to undo.
            return;
        }
        // On EXT1 chips the prepare hooks hold and RTC-route the pins only inside
        // esp_deep_sleep_try_to_start, so a software reject (no returned code) left nothing
        // to undo. On ESP32-C2/C3 `configure` already set the per-pin wake enable (and the
        // sleep-select), so the disarm must run on a software reject too.
        let c2_c3 = cfg!(all(
            esp_idf_soc_gpio_support_deepsleep_wakeup,
            not(esp_idf_soc_pm_support_ext1_wakeup)
        ));
        if self.returned_code.is_some() || c2_c3 {
            release_wake_pins(pins);
        }
        if self.returned_code.is_none() {
            log::warn!(
                "Sleep: wake pin already at its wake level; rejecting in software \
                 (the hardware would hang or sleep and wake at once)"
            );
        }
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
/// # Low-level EXT1 wakes on the ESP32
///
/// On the original ESP32, ESP-IDF's boot code clears the EXT1 pin selection before
/// `app_main`, and after a low-level (`AnyLow`) wake the status then reads empty.
/// For a sleep armed through [`EspSleepManager`] this source falls back to the armed
/// configuration, kept in RTC slow memory: the ESP32 refuses multi-pin `AnyLow`, so a
/// low-level wake there always means the one armed pin.
/// The first call in a boot consumes the record, so a later sleep armed by raw ESP-IDF calls
/// does not inherit it and reports `Ext1` with an empty mask; repeated calls in one boot agree.
/// Keep RTC slow memory powered in deep sleep (do not force `ESP_PD_DOMAIN_RTC_SLOW_MEM` off),
/// or the record is lost; a reset that powers RTC memory down (power-on, brownout) is not an
/// EXT1 wake, so it never consults the record.
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
                // Recommended to call early in main() before peripheral init.
                let status = unsafe { sys::esp_sleep_get_ext1_wakeup_status() };
                // On the ESP32 a low-level wake reads an empty status here; the record of
                // the armed configuration names the single pin (see `armed_ext1`).
                #[cfg(esp32)]
                let mask = {
                    let (armed_mask, armed_low) = armed_ext1::take().unwrap_or((0, false));
                    stoker::ext1_fired_mask(status, armed_mask, armed_low)
                };
                #[cfg(not(esp32))]
                let mask = GpioWakeMask(status);
                WakeCause::Ext1(mask)
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
