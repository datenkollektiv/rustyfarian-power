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
//! - [`EspChargingMonitor`] — resolves charging state from MCP73831 STAT + USB VBUS pins
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

// Re-export the platform-agnostic surface so consumers import from one crate.
pub use stoker::{
    BatteryConfig, BatteryMonitor, BatteryStatus, ChargingMonitor, ChargingSource, ChargingState,
    GpioWakeLevel, GpioWakeMask, NoopBatteryMonitor, NoopChargingMonitor, NoopSleepManager,
    PowerSource, SleepManager, WakeCause, WakeCauseSource, WakeSource,
};

mod esp_adc;
mod esp_charging;
mod esp_sleep;

pub use esp_adc::EspAdcBatteryMonitor;
pub use esp_charging::EspChargingMonitor;
pub use esp_sleep::{EspSleepManager, EspWakeCauseSource};
