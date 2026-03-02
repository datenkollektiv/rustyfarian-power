//! ESP-IDF ADC battery monitor implementation.
//!
//! Reads battery voltage via an ADC pin with voltage divider compensation.
//! On Heltec WiFi LoRa 32 V3, battery voltage is on GPIO1 (ADC1_CH0)
//! through a 2:1 voltage divider.

use esp_idf_hal::adc::attenuation::DB_11;
use esp_idf_hal::adc::oneshot::config::AdcChannelConfig;
use esp_idf_hal::adc::oneshot::{AdcChannelDriver, AdcDriver};
use esp_idf_hal::adc::ADC1;
use esp_idf_hal::gpio::Gpio1;
use esp_idf_hal::peripheral::Peripheral;

use crate::{BatteryConfig, BatteryMonitor, BatteryStatus};

/// Battery monitor using ESP-IDF ADC oneshot driver.
///
/// Reads battery voltage from GPIO1 (ADC1 channel 0) with a configurable
/// voltage divider ratio and sample averaging.
pub struct EspAdcBatteryMonitor<'a> {
    channel: AdcChannelDriver<'a, Gpio1, AdcDriver<'a, ADC1>>,
    config: BatteryConfig,
}

impl<'a> EspAdcBatteryMonitor<'a> {
    /// Creates a new battery monitor using ADC1 / GPIO1.
    ///
    /// 11 dB attenuation is selected for the 0–3100 mV measurement range, which
    /// comfortably covers the 0–2100 mV raw signal produced by the 2:1 voltage
    /// divider on Heltec V3 GPIO1 (4200 mV battery max → 2100 mV at ADC pin).
    pub fn new(
        adc1: impl Peripheral<P = ADC1> + 'a,
        battery_pin: impl Peripheral<P = Gpio1> + 'a,
        config: BatteryConfig,
    ) -> Result<Self, esp_idf_hal::sys::EspError> {
        let adc = AdcDriver::new(adc1)?;

        let channel_config = AdcChannelConfig {
            attenuation: DB_11,
            ..Default::default()
        };
        let channel = AdcChannelDriver::new(adc, battery_pin, &channel_config)?;

        log::info!(
            "Battery monitor initialized (divider: {}x, range: {}-{}mV)",
            config.divider_ratio,
            config.min_voltage_mv,
            config.max_voltage_mv
        );

        Ok(Self { channel, config })
    }

    /// Read averaged ADC voltage in millivolts (before divider compensation).
    fn read_averaged_mv(&mut self) -> u16 {
        let num_samples = self.config.samples.max(1) as u32;
        let mut sum: u32 = 0;
        let mut valid_count: u32 = 0;

        for _ in 0..num_samples {
            match self.channel.read() {
                Ok(value) => {
                    sum += value as u32;
                    valid_count += 1;
                }
                Err(e) => {
                    log::warn!("ADC read failed: {:?}", e);
                }
            }
        }

        if valid_count == 0 {
            log::error!("All ADC reads failed");
            return 0;
        }

        (sum / valid_count) as u16
    }
}

impl<'a> BatteryMonitor for EspAdcBatteryMonitor<'a> {
    /// Read the current battery status.
    ///
    /// Takes multiple ADC samples, averages them, and delegates to
    /// [`BatteryConfig::evaluate_reading()`] for conversion.
    fn read(&mut self) -> BatteryStatus {
        let raw_mv = self.read_averaged_mv();
        let status = self.config.evaluate_reading(raw_mv);
        log::debug!("Battery: {}", status);
        status
    }
}
