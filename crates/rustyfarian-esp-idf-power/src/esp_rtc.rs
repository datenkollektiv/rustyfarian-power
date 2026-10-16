//! RTC clock for slept-time accounting on ESP-IDF.
//!
//! [`EspRtcClock::now_us`] reads the RTC timer, which keeps counting through deep
//! sleep, so firmware can store the time at sleep entry in a retained record and
//! subtract it on wake to age a session.
//! `esp_timer_get_time()` cannot do this: its timebase restarts at every boot.
//!
//! # Semantics
//!
//! The value is microseconds since the RTC timer last started, which is the last
//! power-on or brownout; it continues through deep sleep, `esp_restart()`, a panic
//! and the watchdogs (bench 2026-10-16, `docs/bench/c3-idf-rtc-retention-bench-v1.md`).
//! It is not wall-clock time; ESP-IDF scales the counter by the slow-clock calibration
//! it measured at boot.
//!
//! # Binding
//!
//! `esp_rtc_get_time_us()` is declared per chip in `esp_hw_support/include/soc/<chip>/rtc.h`
//! and is not in the esp-idf-sys 0.38 bindings, so this module declares it itself
//! (linked and run on the C3; the header declares it for the ESP32 and ESP32-S3 as well).
//!
//! # Example
//!
//! ```rust,ignore
//! use rustyfarian_esp_idf_power::EspRtcClock;
//!
//! let slept_us = EspRtcClock::now_us().saturating_sub(retained_sleep_entry_us);
//! ```

extern "C" {
    /// `uint64_t esp_rtc_get_time_us(void)` from `soc/<chip>/rtc.h` (ESP-IDF v5.3.3).
    fn esp_rtc_get_time_us() -> u64;
}

/// The RTC timer as a monotonic microsecond clock that survives deep sleep.
///
/// A unit struct for symmetry with [`EspWakeCauseSource`](crate::EspWakeCauseSource);
/// the clock has no state of its own.
pub struct EspRtcClock;

impl EspRtcClock {
    /// Microseconds on the RTC timer since the last power-on or brownout.
    ///
    /// See the [module docs](self) for what survives which reset.
    #[must_use]
    pub fn now_us() -> u64 {
        // SAFETY: esp_rtc_get_time_us() only reads the RTC timer registers and the
        // calibration ESP-IDF stored at boot; it takes no arguments and changes no
        // hardware state. The declaration matches the ESP-IDF v5.3.3 prototype.
        unsafe { esp_rtc_get_time_us() }
    }
}
