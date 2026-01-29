//! Battery voltage monitoring for embedded projects.
//!
//! Provides battery voltage reading, percentage estimation,
//! and power source detection (battery vs USB).
//!
//! # ESP-IDF (default feature)
//!
//! The `esp-idf` feature provides [`EspAdcBatteryMonitor`], which reads
//! battery voltage via an ADC pin with a configurable voltage divider.

pub mod config;

#[cfg(feature = "esp-idf")]
pub mod esp_adc;

pub use config::BatteryConfig;

#[cfg(feature = "esp-idf")]
pub use esp_adc::EspAdcBatteryMonitor;

/// Power source detected by the battery monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerSource {
    /// Running on battery power.
    Battery,
    /// Running on USB/external power (no battery or fully charged on USB).
    External,
    /// Cannot determine power source (ADC read failed or not configured).
    Unknown,
}

/// Battery status snapshot.
#[derive(Debug, Clone)]
pub struct BatteryStatus {
    /// Battery voltage in millivolts (after voltage divider compensation).
    pub voltage_mv: u16,
    /// Estimated battery percentage (0-100), or `None` if not determinable.
    pub percentage: Option<u8>,
    /// Detected power source.
    pub power_source: PowerSource,
}

impl BatteryStatus {
    /// Returns `true` if the battery level is sufficient for the given thresholds.
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
            PowerSource::Unknown => write!(f, "Unknown"),
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
        assert_eq!(format!("{}", status), "Unknown");
    }
}
