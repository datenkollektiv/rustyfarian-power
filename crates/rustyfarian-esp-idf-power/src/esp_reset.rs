//! ESP-IDF reset-reason driver.
//!
//! [`EspResetReasonSource`] implements [`stoker::ResetReasonSource`] on top of
//! `esp_reset_reason()`, so firmware can tell a cold power-on from a brownout,
//! a watchdog, a panic or a software reset.
//! The retention envelope ([`stoker::Retained::load`]) needs this to decide whether
//! RTC memory can be trusted.
//!
//! # When to call
//!
//! Call [`last_reset_reason`][ResetReasonSource::last_reset_reason] as the first thing in
//! `main()`, next to [`last_wake_cause`][stoker::WakeCauseSource::last_wake_cause],
//! and before loading any retained state.
//!
//! # Mapping
//!
//! The mapping table lives in the `stoker::reset` module docs, which are the single
//! source of truth.
//! The wildcard arm maps everything the table does not name to [`ResetReason::Other`]:
//! `ESP_RST_EXT`, `ESP_RST_SDIO`, USB, JTAG, eFuse, CPU lockup and any code added by a future ESP-IDF.
//! `ESP_RST_PWR_GLITCH` maps to [`ResetReason::Brownout`] because a power glitch may corrupt RTC memory.
//!
//! # Example
//!
//! ```rust,ignore
//! use rustyfarian_esp_idf_power::{
//!     EspResetReasonSource, EspWakeCauseSource, ResetReasonSource, WakeCauseSource,
//! };
//!
//! let reset = EspResetReasonSource.last_reset_reason();
//! let wake = EspWakeCauseSource.last_wake_cause();
//! ```

use esp_idf_hal::sys;
use stoker::{ResetReason, ResetReasonSource};

/// Reads the reset reason from ESP-IDF.
///
/// # When to call
///
/// Call [`last_reset_reason`][ResetReasonSource::last_reset_reason] first in `main()`,
/// before loading retained state.
/// The value is latched at boot, so calling it later returns the same answer.
///
/// # Cold boot behaviour
///
/// `esp_reset_reason()` returns `ESP_RST_POWERON` after a power-on and
/// `ESP_RST_UNKNOWN` when it cannot tell; both map to [`ResetReason::PowerOn`].
pub struct EspResetReasonSource;

impl ResetReasonSource for EspResetReasonSource {
    fn last_reset_reason(&self) -> ResetReason {
        // SAFETY: esp_reset_reason() reads a value latched at boot and does not
        // change any hardware state. Safe to call at any time.
        let reason = unsafe { sys::esp_reset_reason() };

        #[allow(non_upper_case_globals)]
        match reason {
            sys::esp_reset_reason_t_ESP_RST_POWERON | sys::esp_reset_reason_t_ESP_RST_UNKNOWN => {
                ResetReason::PowerOn
            }
            sys::esp_reset_reason_t_ESP_RST_BROWNOUT
            | sys::esp_reset_reason_t_ESP_RST_PWR_GLITCH => ResetReason::Brownout,
            sys::esp_reset_reason_t_ESP_RST_DEEPSLEEP => ResetReason::DeepSleep,
            sys::esp_reset_reason_t_ESP_RST_SW => ResetReason::Software,
            sys::esp_reset_reason_t_ESP_RST_PANIC => ResetReason::Panic,
            sys::esp_reset_reason_t_ESP_RST_INT_WDT
            | sys::esp_reset_reason_t_ESP_RST_TASK_WDT
            | sys::esp_reset_reason_t_ESP_RST_WDT => ResetReason::Watchdog,
            _ => ResetReason::Other,
        }
    }
}
