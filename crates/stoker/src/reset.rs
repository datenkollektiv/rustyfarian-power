//! Reset-reason reporting and host-side no-op implementation.
//!
//! # Architecture
//!
//! Resets and sleep are separate concerns.
//! [`crate::WakeCauseSource`] answers "why did the device wake from sleep?" and
//! folds every non-sleep reset into [`crate::WakeCause::PowerOn`].
//! [`ResetReasonSource`] answers "how did the last reset happen?" so firmware can
//! tell a cold power-on from a brownout, a watchdog or a software reset.
//!
//! The main consumer is [`crate::Retained::load`], which refuses a retention
//! record after [`ResetReason::PowerOn`] and [`ResetReason::Brownout`] because
//! RTC memory is arbitrary after power loss and may be partially written after
//! a brownout.
//!
//! # Platform mappings
//!
//! Platform tiers map their reset codes onto [`ResetReason`] as follows; this
//! table is the single source of truth, and a tier that deviates documents it
//! on its own `ResetReasonSource` type.
//!
//! | `ResetReason` | ESP-IDF `esp_reset_reason()`                                   | esp-hal `rtc_cntl::reset_reason` (`SocResetReason`)              |
//! |:--------------|:---------------------------------------------------------------|:-----------------------------------------------------------------|
//! | `PowerOn`     | `ESP_RST_POWERON`, `ESP_RST_UNKNOWN`                           | `ChipPowerOn`, `None`                                            |
//! | `Brownout`    | `ESP_RST_BROWNOUT`, `ESP_RST_PWR_GLITCH`                       | `SysBrownOut`, `CorePwrGlitch`                                   |
//! | `DeepSleep`   | `ESP_RST_DEEPSLEEP`                                            | `CoreDeepSleep`                                                  |
//! | `Software`    | `ESP_RST_SW`                                                   | `CoreSw`, `Cpu0Sw` (C3), `CpuSw` (S3)                            |
//! | `Panic`       | `ESP_RST_PANIC`                                                | not reported, see below                                          |
//! | `Watchdog`    | `ESP_RST_INT_WDT`, `ESP_RST_TASK_WDT`, `ESP_RST_WDT`           | every `*Wdt*` variant (`Core*`, `Sys*`, C3 `Cpu0*`, S3 `Cpu*`)   |
//! | `Other`       | `ESP_RST_EXT`, `ESP_RST_SDIO`, USB, JTAG, eFuse, CPU lockup    | everything else (USB, JTAG, eFuse CRC, clock glitch)             |
//!
//! A power glitch maps to `Brownout` on both tiers (`ESP_RST_PWR_GLITCH`,
//! `CorePwrGlitch`) because it may corrupt RTC memory the same way.
//! ESP-IDF v5.3.3 reports `ESP_RST_PWR_GLITCH` only on chips whose port decodes
//! that hardware code; on the C3 and S3 it arrives as `ESP_RST_UNKNOWN`, which is
//! `PowerOn` and refused as well.
//!
//! **`Panic` is best-effort.**
//! ESP-IDF records a panic-handler reset separately from a clean `esp_restart()`.
//! Bare-metal esp-hal has no such code: a panic handler that restarts through
//! `software_reset()` arrives as [`ResetReason::Software`], and one that halts
//! produces a watchdog reset.
//! Consumers must therefore treat `Panic` as a refinement of `Software`, never
//! as the only signal that a panic happened; [`crate::Retained::load`] handles
//! both identically.
//!
//! The ESP-IDF driver lives in the companion `rustyfarian-esp-idf-power` crate.

/// How the device last reset.
///
/// Read this via [`ResetReasonSource::last_reset_reason`] at the start of
/// `main()`, typically alongside [`crate::WakeCause`].
///
/// `#[non_exhaustive]`: platform tiers may gain variants (a USB reset, a
/// CPU-local reset) without a breaking release, so downstream `match` arms
/// need a wildcard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResetReason {
    /// Cold power-on, or the reason could not be determined.
    ///
    /// RTC memory holds arbitrary bits after a power-on.
    PowerOn,
    /// Brownout detector reset, or a power-glitch reset on platforms that report one.
    ///
    /// RTC memory may hold a partially written record.
    Brownout,
    /// Wake from deep sleep.
    DeepSleep,
    /// Clean software reset (`esp_restart()`, `software_reset()`).
    ///
    /// RTC memory is intact.
    Software,
    /// Reset by the panic handler.
    ///
    /// RTC memory is intact, but a `store` may have been interrupted; the
    /// record's checksum decides.
    /// Only platforms that record panic resets report this; others report
    /// [`ResetReason::Software`] (see the [module docs](self)).
    Panic,
    /// Any watchdog reset (task, interrupt or RTC watchdog).
    Watchdog,
    /// Any other reset cause (external pin, JTAG, USB, SDIO, and similar).
    Other,
}

/// ESP-IDF v5.3.3 `esp_reset_reason_t` values, in `esp_system.h` order.
///
/// [`from_esp_idf_code`] maps them; the ESP-IDF tier pins its generated
/// `esp_reset_reason_t_ESP_RST_*` constants to these at compile time, so an
/// upstream renumbering fails the build instead of mis-mapping silently.
pub mod esp_idf_code {
    /// `ESP_RST_UNKNOWN`: the reason could not be determined.
    pub const UNKNOWN: u32 = 0;
    /// `ESP_RST_POWERON`.
    pub const POWERON: u32 = 1;
    /// `ESP_RST_EXT`: external pin.
    pub const EXT: u32 = 2;
    /// `ESP_RST_SW`: `esp_restart()`.
    pub const SW: u32 = 3;
    /// `ESP_RST_PANIC`: panic handler.
    pub const PANIC: u32 = 4;
    /// `ESP_RST_INT_WDT`: interrupt watchdog.
    pub const INT_WDT: u32 = 5;
    /// `ESP_RST_TASK_WDT`: task watchdog.
    pub const TASK_WDT: u32 = 6;
    /// `ESP_RST_WDT`: other watchdogs.
    pub const WDT: u32 = 7;
    /// `ESP_RST_DEEPSLEEP`.
    pub const DEEPSLEEP: u32 = 8;
    /// `ESP_RST_BROWNOUT`.
    pub const BROWNOUT: u32 = 9;
    /// `ESP_RST_SDIO`.
    pub const SDIO: u32 = 10;
    /// `ESP_RST_USB`.
    pub const USB: u32 = 11;
    /// `ESP_RST_JTAG`.
    pub const JTAG: u32 = 12;
    /// `ESP_RST_EFUSE`: eFuse error.
    pub const EFUSE: u32 = 13;
    /// `ESP_RST_PWR_GLITCH`.
    pub const PWR_GLITCH: u32 = 14;
    /// `ESP_RST_CPU_LOCKUP`: double exception.
    pub const CPU_LOCKUP: u32 = 15;
}

/// Map an ESP-IDF `esp_reset_reason()` code onto [`ResetReason`] per the
/// [module docs](self) table.
///
/// Pure and `const`, so the mapping is host-testable here while the ESP-IDF
/// tier only forwards the code it read; unknown or future codes map to
/// [`ResetReason::Other`], which trusts the record's checksum.
///
/// # Examples
///
/// ```
/// use stoker::reset::{esp_idf_code, from_esp_idf_code};
/// use stoker::ResetReason;
///
/// assert_eq!(from_esp_idf_code(esp_idf_code::DEEPSLEEP), ResetReason::DeepSleep);
/// assert_eq!(from_esp_idf_code(esp_idf_code::PWR_GLITCH), ResetReason::Brownout);
/// assert_eq!(from_esp_idf_code(99), ResetReason::Other);
/// ```
#[must_use]
pub const fn from_esp_idf_code(code: u32) -> ResetReason {
    match code {
        esp_idf_code::UNKNOWN | esp_idf_code::POWERON => ResetReason::PowerOn,
        esp_idf_code::BROWNOUT | esp_idf_code::PWR_GLITCH => ResetReason::Brownout,
        esp_idf_code::DEEPSLEEP => ResetReason::DeepSleep,
        esp_idf_code::SW => ResetReason::Software,
        esp_idf_code::PANIC => ResetReason::Panic,
        esp_idf_code::INT_WDT | esp_idf_code::TASK_WDT | esp_idf_code::WDT => ResetReason::Watchdog,
        _ => ResetReason::Other,
    }
}

/// Reads the reason for the most recent reset.
///
/// Mirrors [`crate::WakeCauseSource`]; see the [module docs](self) for why the
/// two are separate traits.
pub trait ResetReasonSource {
    /// Return the reason for the most recent reset.
    ///
    /// Returns [`ResetReason::PowerOn`] on a cold boot and whenever the reason
    /// cannot be determined.
    fn last_reset_reason(&self) -> ResetReason;
}

/// A no-op reset-reason source for host-side unit testing.
///
/// Returns the reason configured at construction; defaults to
/// [`ResetReason::PowerOn`].
///
/// # Examples
///
/// ```
/// use stoker::{NoopResetReasonSource, ResetReason, ResetReasonSource};
///
/// let cold = NoopResetReasonSource::default();
/// assert_eq!(cold.last_reset_reason(), ResetReason::PowerOn);
///
/// let woke = NoopResetReasonSource::with_reason(ResetReason::DeepSleep);
/// assert_eq!(woke.last_reset_reason(), ResetReason::DeepSleep);
/// ```
pub struct NoopResetReasonSource {
    reason: ResetReason,
}

impl Default for NoopResetReasonSource {
    fn default() -> Self {
        Self {
            reason: ResetReason::PowerOn,
        }
    }
}

impl NoopResetReasonSource {
    /// Create a mock that returns the given `ResetReason` from `last_reset_reason()`.
    pub fn with_reason(reason: ResetReason) -> Self {
        Self { reason }
    }
}

impl ResetReasonSource for NoopResetReasonSource {
    fn last_reset_reason(&self) -> ResetReason {
        self.reason
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_reset_reason_defaults_to_power_on() {
        let src = NoopResetReasonSource::default();
        assert_eq!(src.last_reset_reason(), ResetReason::PowerOn);
    }

    #[test]
    fn noop_reset_reason_returns_configured_reason() {
        let src = NoopResetReasonSource::with_reason(ResetReason::Watchdog);
        assert_eq!(src.last_reset_reason(), ResetReason::Watchdog);
    }

    #[test]
    fn reset_reason_variants_are_distinct() {
        assert_ne!(ResetReason::PowerOn, ResetReason::Brownout);
        assert_ne!(ResetReason::Brownout, ResetReason::DeepSleep);
        assert_ne!(ResetReason::DeepSleep, ResetReason::Software);
        assert_ne!(ResetReason::Software, ResetReason::Panic);
        assert_ne!(ResetReason::Panic, ResetReason::Watchdog);
        assert_ne!(ResetReason::Watchdog, ResetReason::Other);
    }

    #[test]
    fn reset_reason_is_copy() {
        let a = ResetReason::DeepSleep;
        let b = a;
        assert_eq!(a, b);
    }

    #[test]
    fn esp_idf_power_on_row() {
        assert_eq!(
            from_esp_idf_code(esp_idf_code::UNKNOWN),
            ResetReason::PowerOn
        );
        assert_eq!(
            from_esp_idf_code(esp_idf_code::POWERON),
            ResetReason::PowerOn
        );
    }

    #[test]
    fn esp_idf_brownout_row() {
        assert_eq!(
            from_esp_idf_code(esp_idf_code::BROWNOUT),
            ResetReason::Brownout
        );
        assert_eq!(
            from_esp_idf_code(esp_idf_code::PWR_GLITCH),
            ResetReason::Brownout
        );
    }

    #[test]
    fn esp_idf_deep_sleep_row() {
        assert_eq!(
            from_esp_idf_code(esp_idf_code::DEEPSLEEP),
            ResetReason::DeepSleep
        );
    }

    #[test]
    fn esp_idf_software_row() {
        assert_eq!(from_esp_idf_code(esp_idf_code::SW), ResetReason::Software);
    }

    #[test]
    fn esp_idf_panic_row() {
        assert_eq!(from_esp_idf_code(esp_idf_code::PANIC), ResetReason::Panic);
    }

    #[test]
    fn esp_idf_watchdog_row() {
        for code in [
            esp_idf_code::INT_WDT,
            esp_idf_code::TASK_WDT,
            esp_idf_code::WDT,
        ] {
            assert_eq!(from_esp_idf_code(code), ResetReason::Watchdog, "{code}");
        }
    }

    #[test]
    fn esp_idf_other_row() {
        for code in [
            esp_idf_code::EXT,
            esp_idf_code::SDIO,
            esp_idf_code::USB,
            esp_idf_code::JTAG,
            esp_idf_code::EFUSE,
            esp_idf_code::CPU_LOCKUP,
        ] {
            assert_eq!(from_esp_idf_code(code), ResetReason::Other, "{code}");
        }
    }

    #[test]
    fn esp_idf_literal_codes_follow_the_header() {
        // Literal values from esp_system.h, independent of the `esp_idf_code` constants.
        let expected = [
            (0, ResetReason::PowerOn),
            (1, ResetReason::PowerOn),
            (2, ResetReason::Other),
            (3, ResetReason::Software),
            (4, ResetReason::Panic),
            (5, ResetReason::Watchdog),
            (6, ResetReason::Watchdog),
            (7, ResetReason::Watchdog),
            (8, ResetReason::DeepSleep),
            (9, ResetReason::Brownout),
            (10, ResetReason::Other),
            (11, ResetReason::Other),
            (12, ResetReason::Other),
            (13, ResetReason::Other),
            (14, ResetReason::Brownout),
            (15, ResetReason::Other),
        ];
        for (code, reason) in expected {
            assert_eq!(from_esp_idf_code(code), reason, "{code}");
        }
    }

    #[test]
    fn esp_idf_out_of_range_is_other() {
        assert_eq!(from_esp_idf_code(16), ResetReason::Other);
        assert_eq!(from_esp_idf_code(u32::MAX), ResetReason::Other);
    }

    #[test]
    fn esp_idf_codes_are_contiguous_in_header_order() {
        let codes = [
            esp_idf_code::UNKNOWN,
            esp_idf_code::POWERON,
            esp_idf_code::EXT,
            esp_idf_code::SW,
            esp_idf_code::PANIC,
            esp_idf_code::INT_WDT,
            esp_idf_code::TASK_WDT,
            esp_idf_code::WDT,
            esp_idf_code::DEEPSLEEP,
            esp_idf_code::BROWNOUT,
            esp_idf_code::SDIO,
            esp_idf_code::USB,
            esp_idf_code::JTAG,
            esp_idf_code::EFUSE,
            esp_idf_code::PWR_GLITCH,
            esp_idf_code::CPU_LOCKUP,
        ];
        for (i, code) in codes.iter().enumerate() {
            assert_eq!(*code as usize, i);
        }
    }
}
