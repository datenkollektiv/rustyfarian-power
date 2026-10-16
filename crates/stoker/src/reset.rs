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
}
