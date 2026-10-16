//! Reset-reason reporting on esp-hal.
//!
//! [`EspHalResetReasonSource`] reads the ROM reset cause of the PRO CPU and maps it onto [`ResetReason`].
//! It mirrors the ESP-IDF `EspResetReasonSource`, and the mapping table in [`stoker::reset`] is the source of truth.
//!
//! # Panic
//!
//! This source never reports [`ResetReason::Panic`].
//! Bare-metal esp-hal records no panic reset: a panic handler that restarts through `software_reset()` arrives as [`ResetReason::Software`].
//! See the `Panic` note in the [`stoker::reset`] table.
//!
//! # When to call
//!
//! The ROM latches the reset cause until the next reset, so no capture is needed and the call can happen at any point in `main`.
//! Unlike the wake cause, it is not overwritten by a later sleep attempt.
//!
//! # Mapping
//!
//! `None` (the ROM code could not be decoded) and `ChipPowerOn` map to [`ResetReason::PowerOn`], like `ESP_RST_UNKNOWN` on ESP-IDF, so `load` distrusts RTC memory.
//! `SysBrownOut` and `CorePwrGlitch` map to [`ResetReason::Brownout`]; `CoreDeepSleep` to [`ResetReason::DeepSleep`].
//! `CoreSw` and the CPU-local software reset (`Cpu0Sw` on the C3, `CpuSw` on the S3) map to [`ResetReason::Software`].
//! Every watchdog variant (`Core*`, `Sys*`, and the chip's `Cpu0*` or `Cpu*` watchdogs) maps to [`ResetReason::Watchdog`].
//! Everything else (clock glitch, eFuse CRC, USB, JTAG) maps to [`ResetReason::Other`], which trusts the record's checksum.

pub use stoker::reset::*;

use esp_hal::rtc_cntl::{reset_reason, SocResetReason};
use esp_hal::system::Cpu;

/// Reads the reset reason from the ROM reset cause (`rtc_cntl::reset_reason`).
///
/// The CPU-local variants differ per chip: the C3 names them `Cpu0*`, the S3 `Cpu*`.
/// The mapping covers the chip selected by the crate features.
///
/// # Examples
///
/// ```rust,ignore
/// use rustyfarian_esp_hal_power::{EspHalResetReasonSource, ResetReasonSource};
///
/// let reset = EspHalResetReasonSource.last_reset_reason();
/// ```
#[derive(Debug, Clone, Copy)]
pub struct EspHalResetReasonSource;

impl ResetReasonSource for EspHalResetReasonSource {
    fn last_reset_reason(&self) -> ResetReason {
        map(reset_reason(Cpu::ProCpu))
    }
}

fn map(reason: Option<SocResetReason>) -> ResetReason {
    use SocResetReason as Soc;
    match reason {
        None | Some(Soc::ChipPowerOn) => ResetReason::PowerOn,
        Some(Soc::SysBrownOut | Soc::CorePwrGlitch) => ResetReason::Brownout,
        Some(Soc::CoreDeepSleep) => ResetReason::DeepSleep,
        Some(Soc::CoreSw) => ResetReason::Software,
        #[cfg(not(feature = "esp32s3"))]
        Some(Soc::Cpu0Sw) => ResetReason::Software,
        #[cfg(feature = "esp32s3")]
        Some(Soc::CpuSw) => ResetReason::Software,
        Some(
            Soc::CoreMwdt0 | Soc::CoreMwdt1 | Soc::CoreRtcWdt | Soc::SysRtcWdt | Soc::SysSuperWdt,
        ) => ResetReason::Watchdog,
        #[cfg(not(feature = "esp32s3"))]
        Some(Soc::Cpu0Mwdt0 | Soc::Cpu0Mwdt1 | Soc::Cpu0RtcWdt) => ResetReason::Watchdog,
        #[cfg(feature = "esp32s3")]
        Some(Soc::CpuMwdt0 | Soc::CpuMwdt1 | Soc::CpuRtcWdt) => ResetReason::Watchdog,
        Some(_) => ResetReason::Other,
    }
}
