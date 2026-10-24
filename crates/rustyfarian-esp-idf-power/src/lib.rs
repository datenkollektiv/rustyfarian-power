//! ESP-IDF (std) battery monitoring, charging detection, and deep-sleep drivers
//! for ESP32 and ESP32-S3.
//!
//! This crate is the hardware tier of the rustyfarian power stack. It provides
//! the ESP-IDF implementations of the traits defined in the platform-agnostic
//! [`stoker`] crate:
//!
//! - [`EspAdcBatteryMonitor`] — reads battery voltage via the ESP-IDF ADC oneshot driver
//! - [`EspSleepManager`] — enters deep sleep with configured wake sources
//! - [`EspWakeCauseSource`] — reads the reason for the last wake
//! - [`EspResetReasonSource`] — reads the reason for the last reset, which gates [`RetainedCell::load`]
//! - [`EspRtcClock`] — the RTC timer in microseconds, which keeps counting through deep sleep
//! - [`GpioRailGate`] — switches a power rail (such as the Heltec V3 `Vext`) from one GPIO
//! - [`EspChargingMonitor`] — resolves charging state from MCP73831 STAT + USB VBUS pins
//!
//! With the default feature `critical-section-impl` the crate also provides the
//! firmware's `critical-section` implementation over the FreeRTOS port critical
//! section, which `RetainedCell` needs to exclude interrupts and the other core;
//! disable the feature if the firmware links another provider (esp-idf-hal's own
//! `critical-section` feature is a task-level mutex and is not interrupt-safe).
//!
//! Everything public in [`stoker`] (the traits, config, state types, and the
//! `Noop*` host mocks) is re-exported here, so device firmware needs only a
//! single import:
//!
//! ```rust,ignore
//! use rustyfarian_esp_idf_power::{BatteryConfig, BatteryMonitor, EspAdcBatteryMonitor};
//! ```
//!
//! # Reading the wake cause
//!
//! Read the wake cause first, before any peripheral initialisation:
//!
//! ```rust,ignore
//! use rustyfarian_esp_idf_power::{
//!     EspWakeCauseSource, EspSleepManager, WakeCause, WakeCauseSource, SleepManager, WakeSource,
//! };
//!
//! fn main() {
//!     // EspWakeCauseSource is a unit struct — construct it as a value, then call the trait method.
//!     let cause = EspWakeCauseSource.last_wake_cause();
//!     match cause {
//!         WakeCause::PowerOn => log::info!("Cold boot"),
//!         WakeCause::Timer => log::info!("Woke from timer"),
//!         _ => log::info!("Other wake: {:?}", cause),
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
//!
//! A complete, compiling example is in
//! [`examples/idf_esp32_battery.rs`](https://github.com/datenkollektiv/rustyfarian-power/blob/main/crates/rustyfarian-esp-idf-power/examples/idf_esp32_battery.rs).
//! It is verified to build against the `xtensa-esp32-espidf` target in CI.
//!
//! Key design notes for device-side usage:
//!
//! - `EspWakeCauseSource` is a **unit struct** — the type name is also a value expression.
//!   `EspWakeCauseSource.last_wake_cause()` constructs the struct and calls the trait method
//!   in one expression; it is equivalent to `let s = EspWakeCauseSource; s.last_wake_cause()`.
//! - Call `last_wake_cause()` early in `main()`, before peripheral initialisation.
//!   The EXT1 status register is hardware-preserved until the next sleep entry.
//! - `EspSleepManager::sleep()` does not return on real hardware.
//!   The next code to execute is the firmware entry point after the device wakes.
//! - `EspSleepManager::try_sleep()` returns `EspSleepError` with `kind() == SleepErrorKind::Rejected`
//!   instead of sleeping when a wake source is already asserted.

// Re-export the platform-agnostic surface so consumers import from one crate.
pub use stoker::{
    BatteryConfig, BatteryMonitor, BatteryStatus, ChargingMonitor, ChargingSource, ChargingState,
    Error, GpioWakeLevel, GpioWakeMask, Invalid, NoopBatteryMonitor, NoopChargingMonitor,
    NoopResetReasonSource, NoopSleepManager, PowerSource, ResetReason, ResetReasonSource,
    Retainable, Retained, RetainedCell, SleepError, SleepErrorKind, SleepManager, StoreError,
    WakeCause, WakeCauseSource, WakeSource, MAX_TIMER_MS,
};
pub use stoker::{
    GateError, GateErrorKind, GatedSleep, GatedSleepError, NoopPowerRailGate, PowerRailGate,
};

mod esp_adc;
mod esp_charging;
#[cfg(feature = "critical-section-impl")]
mod esp_critical_section;
mod esp_gate;
mod esp_reset;
mod esp_rtc;
mod esp_sleep;

pub use esp_adc::EspAdcBatteryMonitor;
pub use esp_charging::EspChargingMonitor;
pub use esp_gate::{GpioRailGate, GpioRailGateError, Polarity, HELTEC_V3_VEXT_GPIO};
pub use esp_reset::EspResetReasonSource;
pub use esp_rtc::EspRtcClock;
pub use esp_sleep::{EspSleepError, EspSleepManager, EspWakeCauseSource};
