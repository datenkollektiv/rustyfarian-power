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
//! use battery_monitor::{EspWakeCauseSource, EspSleepManager, WakeCause, WakeCauseSource,
//!                       SleepManager, WakeSource};
//!
//! fn main() {
//!     let cause = EspWakeCauseSource.last_wake_cause();
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

use anyhow::Context;
use esp_idf_hal::sys;

use crate::sleep::{SleepManager, WakeCause, WakeCauseSource, WakeSource};

/// Converts an `esp_err_t` return value into `anyhow::Result<()>`.
fn check(err: sys::esp_err_t) -> anyhow::Result<()> {
    if err != sys::ESP_OK {
        anyhow::bail!("ESP-IDF error: 0x{:08x}", err)
    }
    Ok(())
}

/// ESP-IDF deep sleep implementation for ESP32 targets.
///
/// Clears any previously configured wake sources, configures the requested
/// sources, optionally isolates GPIO pins, then enters deep sleep via
/// `esp_deep_sleep_start()`.
///
/// **[`sleep`][SleepManager::sleep] does not return on real hardware.**
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
    /// before `esp_deep_sleep_start()` to prevent floating inputs from leaking
    /// current during sleep.
    pub isolate_gpio: bool,
}

impl Default for EspSleepManager {
    fn default() -> Self {
        Self { isolate_gpio: true }
    }
}

impl SleepManager for EspSleepManager {
    /// Clear previous wake sources, configure new ones, and enter deep sleep.
    ///
    /// All previously registered wake sources are disabled first via
    /// `esp_sleep_disable_wakeup_source(ALL)`, giving callers deterministic
    /// "what I pass in is what I get" semantics across multiple sleep cycles.
    ///
    /// **This function does not return on real hardware.**
    ///
    /// # Errors
    ///
    /// - More than one `WakeSource::Timer` entry is an error — ESP-IDF supports
    ///   only one timer wake source; repeated calls overwrite each other.
    /// - Returns an error if any ESP-IDF wake-source configuration call fails.
    /// - The device does not enter sleep if an error is returned.
    fn sleep(&mut self, sources: &[WakeSource]) -> anyhow::Result<()> {
        let timer_count = sources
            .iter()
            .filter(|s| matches!(s, WakeSource::Timer { .. }))
            .count();
        if timer_count > 1 {
            anyhow::bail!(
                "at most one Timer wake source is supported per sleep call; {} were provided",
                timer_count
            );
        }

        // Clear any wake sources left over from a previous sleep cycle so the
        // caller gets deterministic behaviour regardless of prior state.
        //
        // SAFETY: esp_sleep_disable_wakeup_source is safe to call at any time
        // before deep sleep entry. Passing ESP_SLEEP_WAKEUP_ALL disables every
        // registered source in one call.
        unsafe {
            check(sys::esp_sleep_disable_wakeup_source(
                sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_ALL,
            ))
            .context("failed to clear previous wake sources")?;
        }

        for source in sources {
            match source {
                WakeSource::Timer { duration_ms } => {
                    let duration_us = duration_ms
                        .checked_mul(1_000)
                        .context("timer duration overflow converting ms to µs")?;
                    // SAFETY: esp_sleep_enable_timer_wakeup configures the RTC
                    // timer wakeup source. Safe to call before deep sleep entry;
                    // duration_us has been validated above.
                    unsafe {
                        check(sys::esp_sleep_enable_timer_wakeup(duration_us))
                            .context("failed to configure timer wakeup")?;
                    }
                    log::info!("Sleep: timer wake configured for {}ms", duration_ms);
                }
            }
        }

        if self.isolate_gpio {
            // SAFETY: esp_sleep_config_gpio_isolate isolates all digital GPIO
            // pins to prevent floating inputs from leaking current during sleep.
            // Returns void in ESP-IDF v5.x. Must be called after wake sources
            // are configured and before esp_deep_sleep_start().
            unsafe {
                sys::esp_sleep_config_gpio_isolate();
            }
        }

        log::info!("Entering deep sleep");

        // SAFETY: All wake sources have been configured above. GPIO pins are
        // isolated (when isolate_gpio is true). esp_deep_sleep_start() never
        // returns — the next execution begins at the firmware entry point when
        // the device wakes.
        unsafe {
            sys::esp_deep_sleep_start();
        }
    }
}

/// Reads the wakeup cause from the ESP-IDF wakeup cause register.
///
/// Instantiate this at the very start of `main()`, before any peripheral
/// initialisation that might clear wake-related hardware state.
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
            sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_EXT0
            | sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_EXT1
            | sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_GPIO => WakeCause::Gpio,
            sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_TOUCHPAD => WakeCause::Touch,
            _ => WakeCause::Other,
        }
    }
}
