//! Battery monitoring configuration.

use crate::{BatteryStatus, PowerSource};

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
    /// Default values are calibrated for the Heltec WiFi LoRa 32 V3.
    /// Use [`BatteryConfig::heltec_v3`] for a more self-documenting constructor on that board.
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
    /// Battery configuration preset for the Heltec WiFi LoRa 32 V3.
    ///
    /// Uses the same values as [`BatteryConfig::default`] — the default was
    /// calibrated against this board, so the preset is provided as an explicit
    /// constructor for readability and future divergence.
    /// If you target a different board, start with this preset and adjust
    /// `divider_ratio`, `usb_detection_mv`, and voltage bounds to match your
    /// hardware.
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

    /// Evaluate a raw ADC reading and produce a [`BatteryStatus`].
    ///
    /// Applies voltage divider compensation, detects the power source, and
    /// calculates battery percentage when running on battery power.
    /// This contains all conversion logic so it can be tested without hardware.
    pub fn evaluate_reading(&self, raw_mv: u16) -> BatteryStatus {
        let voltage_mv = (raw_mv as f32 * self.divider_ratio) as u16;

        let power_source = if voltage_mv > self.usb_detection_mv {
            PowerSource::External
        } else if voltage_mv < self.min_voltage_mv / 2 {
            PowerSource::Unknown
        } else {
            PowerSource::Battery
        };

        let percentage = match power_source {
            PowerSource::Battery => Some(self.voltage_to_percent(voltage_mv)),
            _ => None,
        };

        BatteryStatus {
            voltage_mv,
            percentage,
            power_source,
        }
    }
}
