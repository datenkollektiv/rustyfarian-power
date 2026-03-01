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
//! Enable the `esp-idf` feature (default) to access [`EspSleepManager`] and
//! [`EspWakeCauseSource`] in the [`crate::esp_sleep`] module.

/// The reason the device woke from sleep, or [`WakeCause::PowerOn`] on cold boot.
///
/// Read this via [`WakeCauseSource::last_wake_cause`] at the start of `main()`
/// to determine whether the device is recovering from a sleep cycle or starting fresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeCause {
    /// Cold power-on or hardware reset — not a wake from sleep.
    ///
    /// Also returned by [`NoopSleepManager`] and whenever the ESP-IDF wakeup
    /// cause is `UNDEFINED`, which occurs on every first boot.
    PowerOn,
    /// Woken by the configured sleep timer.
    Timer,
    /// Woken by a GPIO signal (EXT0, EXT1, or deep-sleep GPIO wakeup).
    Gpio,
    /// Woken by a touch sensor.
    Touch,
    /// Woken by some other cause (ULP, UART, Wi-Fi, Bluetooth, etc.).
    Other,
}

/// A wake source to configure before entering sleep.
#[derive(Debug, Clone, Copy)]
pub enum WakeSource {
    /// Wake the device after the given number of milliseconds.
    Timer {
        /// Wake delay in milliseconds.
        duration_ms: u64,
    },
}

/// Controls entry into a low-power sleep mode.
///
/// # Does not return on real hardware
///
/// On ESP32 targets, `sleep()` calls `esp_deep_sleep_start()`, which never
/// returns.
/// The firmware restarts from the entry point at the next boot.
/// On host (mock) targets the call is a no-op and returns `Ok(())`.
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
    /// Returns [`WakeCause::PowerOn`] on first boot (cold power-on) and whenever
    /// the cause cannot be determined.
    fn last_wake_cause(&self) -> WakeCause;
}

/// A no-op sleep manager for host-side unit testing.
///
/// [`SleepManager::sleep`] returns `Ok(())` immediately without entering any
/// sleep mode.
/// [`WakeCauseSource::last_wake_cause`] always returns [`WakeCause::PowerOn`].
pub struct NoopSleepManager;

impl SleepManager for NoopSleepManager {
    fn sleep(&mut self, _sources: &[WakeSource]) -> anyhow::Result<()> {
        Ok(())
    }
}

impl WakeCauseSource for NoopSleepManager {
    fn last_wake_cause(&self) -> WakeCause {
        WakeCause::PowerOn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_sleep_returns_ok() {
        let mut mgr = NoopSleepManager;
        assert!(mgr
            .sleep(&[WakeSource::Timer { duration_ms: 5000 }])
            .is_ok());
    }

    #[test]
    fn noop_sleep_empty_sources_returns_ok() {
        let mut mgr = NoopSleepManager;
        assert!(mgr.sleep(&[]).is_ok());
    }

    #[test]
    fn noop_wake_cause_is_power_on() {
        let mgr = NoopSleepManager;
        assert_eq!(mgr.last_wake_cause(), WakeCause::PowerOn);
    }

    #[test]
    fn wake_cause_variants_are_distinct() {
        assert_ne!(WakeCause::PowerOn, WakeCause::Timer);
        assert_ne!(WakeCause::Timer, WakeCause::Gpio);
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
        let WakeSource::Timer { duration_ms } = src;
        assert_eq!(duration_ms, 60_000);
    }
}
