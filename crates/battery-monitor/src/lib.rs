//! Battery voltage monitoring and power management for embedded projects.
//!
//! Provides battery voltage reading, percentage estimation, power source
//! detection (battery vs USB), and deep sleep management.
//!
//! # ESP-IDF (default feature)
//!
//! The `esp-idf` feature provides hardware implementations:
//! - [`EspAdcBatteryMonitor`] — reads battery voltage via ADC
//! - [`EspSleepManager`] — enters deep sleep with configured wake sources
//! - [`EspWakeCauseSource`] — reads the reason for the last wake
//!
//! # Quick start
//!
//! ## Host-side testing (no ESP toolchain required)
//!
//! Use [`NoopSleepManager`] and [`NoopBatteryMonitor`] to test logic that
//! branches on wake cause or battery state — no hardware needed:
//!
//! ```
//! use battery_monitor::{
//!     BatteryMonitor, NoopBatteryMonitor,
//!     NoopSleepManager, SleepManager, WakeCauseSource, WakeCause, WakeSource,
//! };
//!
//! // Simulate a timer wake and check that consumer code handles it.
//! let wake_src = NoopSleepManager::with_cause(WakeCause::Timer);
//! assert_eq!(wake_src.last_wake_cause(), WakeCause::Timer);
//!
//! // Simulate a battery reading and verify the sufficiency check.
//! let mut battery = NoopBatteryMonitor::on_battery(3700, 60);
//! assert!(battery.read().is_sufficient(3600, 20));
//!
//! // Sleep configuration errors are surfaced just as on device.
//! let mut mgr = NoopSleepManager::default();
//! mgr.sleep(&[WakeSource::Timer { duration_ms: 60_000 }]).unwrap();
//! ```
//!
//! ## Device firmware (requires `esp-idf` feature)
//!
//! The `esp-idf` types (`EspAdcBatteryMonitor`, `EspWakeCauseSource`, `EspSleepManager`)
//! are only available when the crate is compiled for an ESP32 target with the default `esp-idf`
//! feature enabled.
//! They cannot be compiled on the host by default, so the snippet below is marked `ignore`.
//! It is kept type-checkable for the ESP-IDF environment; run `just check-docs-esp32` to verify.
//!
//! ```rust,ignore
//! // Ignored by host-side doctest runs.
//! // Type-checks when compiled with `--features esp-idf` for an ESP32 target.
//! use battery_monitor::{EspWakeCauseSource, WakeCauseSource};
//!
//! // `EspWakeCauseSource` is a unit struct; the type name is also a value expression.
//! // Equivalent to: let s = EspWakeCauseSource; s.last_wake_cause()
//! let cause = EspWakeCauseSource.last_wake_cause();
//! let _ = cause;
//! ```
//!
//! A complete, compiling example is in
//! [`examples/idf_esp32_battery.rs`](https://github.com/datenkollektiv/rustyfarian-power/blob/main/crates/battery-monitor/examples/idf_esp32_battery.rs).
//! It demonstrates wake-cause detection, battery ADC reading, charging state, and deep sleep
//! on the Adafruit ESP32 Feather V2, and is verified to build against the `xtensa-esp32-espidf`
//! target in CI.
//!
//! Key design notes for device-side usage:
//!
//! - `EspWakeCauseSource` is a **unit struct** — the type name is also a value expression.
//!   `EspWakeCauseSource.last_wake_cause()` constructs the struct and calls the trait method
//!   in one expression; it is equivalent to `let s = EspWakeCauseSource; s.last_wake_cause()`.
//!   If `EspWakeCauseSource` ever stops being a unit struct, update all call sites to instantiate explicitly.
//! - Call `last_wake_cause()` early in `main()`, before peripheral initialisation.
//!   The EXT1 status register is hardware-preserved until the next sleep entry.
//! - `EspSleepManager::sleep()` does not return on real hardware.
//!   The next code to execute is the firmware entry point after the device wakes.

pub mod charging;
pub mod config;
pub mod sleep;

#[cfg(feature = "esp-idf")]
pub mod esp_adc;

#[cfg(feature = "esp-idf")]
pub mod esp_charging;

#[cfg(feature = "esp-idf")]
pub mod esp_sleep;

pub use charging::{ChargingMonitor, ChargingSource, ChargingState, NoopChargingMonitor};
pub use config::BatteryConfig;
pub use sleep::{
    GpioWakeLevel, GpioWakeMask, NoopSleepManager, SleepManager, WakeCause, WakeCauseSource,
    WakeSource,
};

#[cfg(feature = "esp-idf")]
pub use esp_adc::EspAdcBatteryMonitor;

#[cfg(feature = "esp-idf")]
pub use esp_charging::EspChargingMonitor;

#[cfg(feature = "esp-idf")]
pub use esp_sleep::{EspSleepManager, EspWakeCauseSource};

/// Trait for reading battery status from hardware.
///
/// Implementors provide a `read()` method that samples the ADC and returns
/// a [`BatteryStatus`]. This abstraction allows consumers to mock the
/// hardware for testing.
pub trait BatteryMonitor {
    /// Read the current battery status.
    fn read(&mut self) -> BatteryStatus;
}

/// A no-op battery monitor for host-side unit testing.
///
/// `BatteryMonitor::read` always returns the status configured at construction.
/// Use the convenience constructors to target the three `PowerSource` branches.
///
/// # Examples
///
/// ```
/// use battery_monitor::{BatteryMonitor, NoopBatteryMonitor};
///
/// fn needs_transmit(monitor: &mut impl BatteryMonitor) -> bool {
///     monitor.read().is_sufficient(3600, 20)
/// }
///
/// assert!(needs_transmit(&mut NoopBatteryMonitor::on_external()));
/// assert!(!needs_transmit(&mut NoopBatteryMonitor::on_battery(3400, 10)));
/// ```
pub struct NoopBatteryMonitor {
    status: BatteryStatus,
}

impl NoopBatteryMonitor {
    /// Creates a mock that returns the given `status` on every `read()`.
    pub fn new(status: BatteryStatus) -> Self {
        Self { status }
    }

    /// Creates a mock in the `Battery` state with the given voltage and percentage.
    pub fn on_battery(voltage_mv: u16, percentage: u8) -> Self {
        Self::new(BatteryStatus {
            voltage_mv,
            percentage: Some(percentage),
            power_source: PowerSource::Battery,
        })
    }

    /// Creates a mock in the `External` (USB/charger) state.
    pub fn on_external() -> Self {
        Self::new(BatteryStatus {
            voltage_mv: 5000,
            percentage: None,
            power_source: PowerSource::External,
        })
    }

    /// Creates a mock in the `Unknown` state (ADC failed or unconfigured).
    pub fn unknown() -> Self {
        Self::new(BatteryStatus {
            voltage_mv: 0,
            percentage: None,
            power_source: PowerSource::Unknown,
        })
    }
}

impl BatteryMonitor for NoopBatteryMonitor {
    fn read(&mut self) -> BatteryStatus {
        self.status
    }
}

/// Power source detected by the battery monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerSource {
    /// Running on battery power.
    Battery,
    /// Running on USB/external power (no battery or fully charged on USB).
    External,
    /// Cannot determine a power source.
    ///
    /// Produced when the compensated ADC voltage falls below [`BatteryConfig::min_voltage_mv`],
    /// which most commonly indicates no battery is attached (the voltage divider floats near 0 V).
    /// Also produced when all ADC samples fail due to a hardware error.
    ///
    /// [`BatteryStatus`] displays this variant as `"No battery"` because an absent battery
    /// is the most frequent real-world cause.
    /// To distinguish hardware errors from absent batteries, enable `log::debug` for the
    /// `battery_monitor` component to see the raw ADC value before divider compensation.
    Unknown,
}

/// Battery status snapshot.
#[derive(Debug, Clone, Copy)]
pub struct BatteryStatus {
    /// Battery voltage in millivolts (after voltage divider compensation).
    pub voltage_mv: u16,
    /// Estimated battery percentage (0-100), or `None` if not determinable.
    pub percentage: Option<u8>,
    /// Detected power source.
    pub power_source: PowerSource,
}

impl BatteryStatus {
    /// Returns `true` if the battery level is enough for the given thresholds.
    ///
    /// When a power source is `External` or `Unknown`, this always returns `true`
    /// (graceful fallback: don't block operations when the battery can't be read).
    pub fn is_sufficient(&self, min_voltage_mv: u16, min_percent: u8) -> bool {
        match self.power_source {
            PowerSource::External | PowerSource::Unknown => true,
            PowerSource::Battery => {
                self.voltage_mv >= min_voltage_mv
                    && self.percentage.map_or(true, |p| p >= min_percent)
            }
        }
    }
}

impl core::fmt::Display for BatteryStatus {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.power_source {
            PowerSource::Battery => {
                if let Some(pct) = self.percentage {
                    write!(f, "{}mV ({}%)", self.voltage_mv, pct)
                } else {
                    write!(f, "{}mV", self.voltage_mv)
                }
            }
            PowerSource::External => write!(f, "USB/Ext"),
            PowerSource::Unknown => write!(f, "No battery"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voltage_to_percent_empty() {
        let config = BatteryConfig::default();
        assert_eq!(config.voltage_to_percent(3000), 0);
        assert_eq!(config.voltage_to_percent(2500), 0);
    }

    #[test]
    fn voltage_to_percent_full() {
        let config = BatteryConfig::default();
        assert_eq!(config.voltage_to_percent(4200), 100);
        assert_eq!(config.voltage_to_percent(4500), 100);
    }

    #[test]
    fn voltage_to_percent_midrange() {
        let config = BatteryConfig::default();
        // 3600mV = (3600-3000)/(4200-3000) * 100 = 600/1200 * 100 = 50%
        assert_eq!(config.voltage_to_percent(3600), 50);
    }

    #[test]
    fn is_sufficient_on_battery() {
        let status = BatteryStatus {
            voltage_mv: 3800,
            percentage: Some(67),
            power_source: PowerSource::Battery,
        };
        assert!(status.is_sufficient(3600, 40));
        assert!(!status.is_sufficient(3900, 40)); // voltage too low
        assert!(!status.is_sufficient(3600, 70)); // percentage too low
    }

    #[test]
    fn is_sufficient_on_usb_always_true() {
        let status = BatteryStatus {
            voltage_mv: 5000,
            percentage: None,
            power_source: PowerSource::External,
        };
        assert!(status.is_sufficient(3600, 40));
        assert!(status.is_sufficient(5000, 100));
    }

    #[test]
    fn is_sufficient_unknown_always_true() {
        let status = BatteryStatus {
            voltage_mv: 0,
            percentage: None,
            power_source: PowerSource::Unknown,
        };
        assert!(status.is_sufficient(3600, 40));
    }

    #[test]
    fn display_battery() {
        let status = BatteryStatus {
            voltage_mv: 3800,
            percentage: Some(67),
            power_source: PowerSource::Battery,
        };
        assert_eq!(format!("{}", status), "3800mV (67%)");
    }

    #[test]
    fn display_external() {
        let status = BatteryStatus {
            voltage_mv: 5000,
            percentage: None,
            power_source: PowerSource::External,
        };
        assert_eq!(format!("{}", status), "USB/Ext");
    }

    #[test]
    fn display_unknown() {
        let status = BatteryStatus {
            voltage_mv: 0,
            percentage: None,
            power_source: PowerSource::Unknown,
        };
        assert_eq!(format!("{}", status), "No battery");
    }

    #[test]
    fn evaluate_reading_battery_midrange() {
        let config = BatteryConfig::default();
        // divider_ratio=2.0, so raw 1800 mV → 3600 mV compensated
        let status = config.evaluate_reading(1800);
        assert_eq!(status.voltage_mv, 3600);
        assert_eq!(status.power_source, PowerSource::Battery);
        assert_eq!(status.percentage, Some(50));
    }

    #[test]
    fn evaluate_reading_battery_full() {
        let config = BatteryConfig::default();
        // raw 2100 → 4200 mV = 100%
        let status = config.evaluate_reading(2100);
        assert_eq!(status.voltage_mv, 4200);
        assert_eq!(status.power_source, PowerSource::Battery);
        assert_eq!(status.percentage, Some(100));
    }

    #[test]
    fn evaluate_reading_battery_empty() {
        let config = BatteryConfig::default();
        // raw 1500 → 3000 mV = 0%
        let status = config.evaluate_reading(1500);
        assert_eq!(status.voltage_mv, 3000);
        assert_eq!(status.power_source, PowerSource::Battery);
        assert_eq!(status.percentage, Some(0));
    }

    #[test]
    fn evaluate_reading_usb_external() {
        let config = BatteryConfig::default();
        // raw 2200 → 4400 mV, above usb_detection_mv (4300)
        let status = config.evaluate_reading(2200);
        assert_eq!(status.voltage_mv, 4400);
        assert_eq!(status.power_source, PowerSource::External);
        assert_eq!(status.percentage, None);
    }

    #[test]
    fn evaluate_reading_very_low_unknown() {
        let config = BatteryConfig::default();
        // raw 500 → 1000 mV, below min_voltage_mv/2 (1500)
        let status = config.evaluate_reading(500);
        assert_eq!(status.voltage_mv, 1000);
        assert_eq!(status.power_source, PowerSource::Unknown);
        assert_eq!(status.percentage, None);
    }

    #[test]
    fn evaluate_reading_zero_unknown() {
        let config = BatteryConfig::default();
        let status = config.evaluate_reading(0);
        assert_eq!(status.voltage_mv, 0);
        assert_eq!(status.power_source, PowerSource::Unknown);
        assert_eq!(status.percentage, None);
    }

    #[test]
    fn evaluate_reading_usb_boundary() {
        let config = BatteryConfig::default();
        // raw 2150 → 4300 mV = exactly usb_detection_mv, not above → Battery
        let status = config.evaluate_reading(2150);
        assert_eq!(status.voltage_mv, 4300);
        assert_eq!(status.power_source, PowerSource::Battery);
        assert_eq!(status.percentage, Some(100));
    }

    #[test]
    fn evaluate_reading_unknown_boundary() {
        let config = BatteryConfig::default();
        // raw 1499 → 2998 mV, just below min_voltage_mv (3000) → Unknown
        let status = config.evaluate_reading(1499);
        assert_eq!(status.voltage_mv, 2998);
        assert_eq!(status.power_source, PowerSource::Unknown);
        assert_eq!(status.percentage, None);
    }

    #[test]
    fn evaluate_reading_custom_divider_ratio() {
        let config = BatteryConfig {
            divider_ratio: 3.0,
            ..BatteryConfig::default()
        };
        // raw 1200 → 3600 mV with 3x divider
        let status = config.evaluate_reading(1200);
        assert_eq!(status.voltage_mv, 3600);
        assert_eq!(status.power_source, PowerSource::Battery);
        assert_eq!(status.percentage, Some(50));
    }

    #[test]
    fn noop_battery_monitor_on_battery_reads_correct_status() {
        let mut mon = NoopBatteryMonitor::on_battery(3700, 60);
        let status = mon.read();
        assert_eq!(status.voltage_mv, 3700);
        assert_eq!(status.percentage, Some(60));
        assert_eq!(status.power_source, PowerSource::Battery);
    }

    #[test]
    fn noop_battery_monitor_on_external_is_always_sufficient() {
        let mut mon = NoopBatteryMonitor::on_external();
        assert!(mon.read().is_sufficient(5000, 100));
    }

    #[test]
    fn noop_battery_monitor_unknown_is_always_sufficient() {
        let mut mon = NoopBatteryMonitor::unknown();
        assert!(mon.read().is_sufficient(5000, 100));
    }

    #[test]
    fn noop_battery_monitor_on_battery_insufficient() {
        let mut mon = NoopBatteryMonitor::on_battery(3400, 10);
        assert!(!mon.read().is_sufficient(3600, 20));
    }
}
