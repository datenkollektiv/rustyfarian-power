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
    /// Generic Li-Po defaults with a 2:1 divider assumption.
    /// Use a board preset ([`BatteryConfig::heltec_v3`], [`BatteryConfig::adafruit_feather_v2`])
    /// for hardware-calibrated values — those override `divider_ratio` for the specific board.
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
    /// Battery configuration preset for the Heltec WiFi LoRa 32 V3 (measured on V3.1).
    ///
    /// Battery voltage is read on **GPIO1** (ADC1_CH0) through an on-board divider
    /// that is **always connected** — GPIO37/ADC_CTRL does *not* gate it on V3.1
    /// (it may on other revisions; confirm against your board).
    ///
    /// `divider_ratio` is **5.55**, an *empirical* value: `metered_VBAT / raw_ADC_mV`
    /// measured on hardware, not the textbook divider figure. It runs higher than the
    /// physical (390 kΩ + 100 kΩ) / 100 kΩ ≈ 4.9 because the ~80 kΩ source impedance
    /// loads the ESP-IDF one-shot ADC, which then reads ~11 % low. Because it folds in
    /// that ADC loading, the ratio is specific to this crate's ADC setup — verify the
    /// reported voltage against a multimeter and nudge `divider_ratio` if your unit differs.
    ///
    /// Recalibrate whenever the ADC attenuation, sampling config, board revision, or VBAT
    /// sense path changes — the ratio absorbs all of those, so a stale value reads wrong.
    ///
    /// Quick procedure:
    /// 1. Meter the real battery voltage at the cell (`metered_VBAT`).
    /// 2. Read the raw pin millivolts before divider compensation (the `read_averaged_mv`
    ///    debug log, or `BatteryStatus.voltage_mv / divider_ratio`).
    /// 3. Set `divider_ratio = metered_VBAT / raw_pin_mV`.
    pub fn heltec_v3() -> Self {
        Self {
            divider_ratio: 5.55,
            max_voltage_mv: 4200,
            min_voltage_mv: 3000,
            usb_detection_mv: 4300,
            samples: 16,
        }
    }

    /// Battery configuration preset for the Adafruit ESP32 Feather V2.
    ///
    /// Targets the original ESP32 chip on the Adafruit Feather V2 board.
    /// The battery voltage is measured on **GPIO35** (ADC1_CH7) after a
    /// 100 kΩ + 100 kΩ voltage divider, giving a `divider_ratio` of 2.0.
    /// Battery chemistry is Li-Po: 3000 mV empty, 4200 mV full.
    /// When powered from USB without a battery attached, the ADC reads above
    /// the Li-Po maximum voltage, so `usb_detection_mv` is set to 4300 mV —
    /// the same threshold used by the Heltec V3 preset.
    pub fn adafruit_feather_v2() -> Self {
        Self {
            divider_ratio: 2.0,
            max_voltage_mv: 4200,
            min_voltage_mv: 3000,
            usb_detection_mv: 4300,
            samples: 16,
        }
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
        } else if voltage_mv < self.min_voltage_mv {
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
