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
//! The driver only reads the code; [`stoker::reset::from_esp_idf_code`] does the
//! mapping, so the table in the `stoker::reset` module docs is host-tested there.
//! The `const` assertions below pin every generated `esp_reset_reason_t_ESP_RST_*`
//! value to the code `stoker` assumes, so an ESP-IDF renumbering fails the build.
//! `ESP_RST_PWR_GLITCH` maps to [`ResetReason::Brownout`] because a power glitch may corrupt RTC memory;
//! codes the table does not name (external pin, SDIO, USB, JTAG, eFuse, CPU lockup) map to [`ResetReason::Other`].
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
use stoker::reset::{esp_idf_code, from_esp_idf_code};
use stoker::{ResetReason, ResetReasonSource};

// Pin the generated constants to the codes `stoker::reset::from_esp_idf_code` assumes.
const _: () = {
    assert!(sys::esp_reset_reason_t_ESP_RST_UNKNOWN as u32 == esp_idf_code::UNKNOWN);
    assert!(sys::esp_reset_reason_t_ESP_RST_POWERON as u32 == esp_idf_code::POWERON);
    assert!(sys::esp_reset_reason_t_ESP_RST_EXT as u32 == esp_idf_code::EXT);
    assert!(sys::esp_reset_reason_t_ESP_RST_SW as u32 == esp_idf_code::SW);
    assert!(sys::esp_reset_reason_t_ESP_RST_PANIC as u32 == esp_idf_code::PANIC);
    assert!(sys::esp_reset_reason_t_ESP_RST_INT_WDT as u32 == esp_idf_code::INT_WDT);
    assert!(sys::esp_reset_reason_t_ESP_RST_TASK_WDT as u32 == esp_idf_code::TASK_WDT);
    assert!(sys::esp_reset_reason_t_ESP_RST_WDT as u32 == esp_idf_code::WDT);
    assert!(sys::esp_reset_reason_t_ESP_RST_DEEPSLEEP as u32 == esp_idf_code::DEEPSLEEP);
    assert!(sys::esp_reset_reason_t_ESP_RST_BROWNOUT as u32 == esp_idf_code::BROWNOUT);
    assert!(sys::esp_reset_reason_t_ESP_RST_SDIO as u32 == esp_idf_code::SDIO);
    assert!(sys::esp_reset_reason_t_ESP_RST_USB as u32 == esp_idf_code::USB);
    assert!(sys::esp_reset_reason_t_ESP_RST_JTAG as u32 == esp_idf_code::JTAG);
    assert!(sys::esp_reset_reason_t_ESP_RST_EFUSE as u32 == esp_idf_code::EFUSE);
    assert!(sys::esp_reset_reason_t_ESP_RST_PWR_GLITCH as u32 == esp_idf_code::PWR_GLITCH);
    assert!(sys::esp_reset_reason_t_ESP_RST_CPU_LOCKUP as u32 == esp_idf_code::CPU_LOCKUP);
};

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
        let code = unsafe { sys::esp_reset_reason() };
        from_esp_idf_code(code as u32)
    }
}
