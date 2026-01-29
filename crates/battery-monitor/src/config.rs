//! Battery monitoring configuration.

/// Configuration for battery voltage monitoring.
#[derive(Debug, Clone)]
pub struct BatteryConfig {
    /// Voltage divider ratio (e.g., 2.0 for a 1:1 resistor divider).
    /// The actual battery voltage = ADC reading * divider_ratio.
    pub divider_ratio: f32,

    /// Maximum battery voltage in millivolts (fully charged).
    /// Default: 4200 mV (Li-ion/LiPo).
    pub max_voltage_mv: u16,

    /// Minimum battery voltage in millivolts (empty, cutoff).
    /// Default: 3000 mV (Li-ion/LiPo safe minimum).
    pub min_voltage_mv: u16,

    /// Voltage above which we consider the device USB-powered.
    /// When no battery is connected, the ADC may read a rail voltage.
    /// Default: 4300 mV (above max Li-ion voltage).
    pub usb_detection_mv: u16,

    /// Number of ADC samples to average for noise reduction.
    /// Default: 16.
    pub samples: u8,
}

impl Default for BatteryConfig {
    fn default() -> Self {
        Self {
            divider_ratio: 2.0,
            max_voltage_mv: 4200,
            min_voltage_mv: 3000,
            usb_detection_mv: 4300,
            samples: 16,
        }
    }
}

impl BatteryConfig {
    /// Create config for Heltec WiFi LoRa 32 V3.
    ///
    /// Uses the default Li-ion values with a 2:1 voltage divider on GPIO1.
    pub fn heltec_v3() -> Self {
        Self::default()
    }

    /// Calculate percentage from a battery voltage (millivolts).
    ///
    /// Uses linear interpolation between min and max voltage.
    /// Returns 0 for voltages at or below min, 100 for voltages at or above max.
    pub fn voltage_to_percent(&self, voltage_mv: u16) -> u8 {
        if voltage_mv <= self.min_voltage_mv {
            return 0;
        }
        if voltage_mv >= self.max_voltage_mv {
            return 100;
        }
        let range = (self.max_voltage_mv - self.min_voltage_mv) as u32;
        let above_min = (voltage_mv - self.min_voltage_mv) as u32;
        ((above_min * 100) / range) as u8
    }
}
