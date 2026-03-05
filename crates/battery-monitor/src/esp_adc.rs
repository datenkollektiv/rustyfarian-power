//! ESP-IDF ADC battery monitor implementation.
//!
//! Reads battery voltage via an ADC pin with voltage divider compensation.
//! Any ADC1-capable GPIO pin is supported; the specific pin depends on the
//! board (e.g. GPIO1 on Heltec WiFi LoRa 32 V3, GPIO35 on Adafruit ESP32 Feather V2).

use esp_idf_hal::adc::attenuation::DB_11;
use esp_idf_hal::adc::oneshot::config::AdcChannelConfig;
#[cfg(esp32)]
use esp_idf_hal::adc::oneshot::config::Calibration;
use esp_idf_hal::adc::oneshot::{AdcChannelDriver, AdcDriver};
use esp_idf_hal::adc::ADC1;
use esp_idf_hal::gpio::ADCPin;
use esp_idf_hal::peripheral::Peripheral;

use crate::{BatteryConfig, BatteryMonitor, BatteryStatus};

/// Battery monitor using ESP-IDF ADC oneshot driver.
///
/// Generic over `GPIO`, which must be an ADC1-capable pin (i.e. `GPIO: ADCPin<Adc = ADC1>`).
/// Use [`BatteryConfig::heltec_v3()`] for the Heltec WiFi LoRa 32 V3 (GPIO1)
/// or construct a custom [`BatteryConfig`] for other boards.
pub struct EspAdcBatteryMonitor<'a, GPIO>
where
    GPIO: ADCPin<Adc = ADC1>,
{
    channel: AdcChannelDriver<'a, GPIO, AdcDriver<'a, ADC1>>,
    config: BatteryConfig,
}

impl<'a, GPIO> EspAdcBatteryMonitor<'a, GPIO>
where
    GPIO: ADCPin<Adc = ADC1>,
{
    /// Creates a new battery monitor using ADC1 and the given ADC1-capable GPIO pin.
    ///
    /// 11 dB attenuation is selected to cover the 0–2100 mV raw signal produced by a 2:1
    /// voltage divider (4200 mV battery max → 2100 mV at the ADC pin).
    /// The effective ceiling is chip-specific: 2450 mV on original ESP32, 3100 mV on ESP32-S3.
    /// Both ceilings comfortably cover the 2100 mV maximum raw signal.
    ///
    /// On original ESP32, `Calibration::Line` is used to correct the ADC's significant
    /// non-linearity at 11 dB attenuation.
    /// Without line-fitting calibration, uncalibrated readings can be hundreds of mV low,
    /// causing a healthy battery to be misclassified as `PowerSource::Unknown`.
    /// On other chips, the default `Calibration::None` is used.
    pub fn new(
        adc1: impl Peripheral<P = ADC1> + 'a,
        battery_pin: impl Peripheral<P = GPIO> + 'a,
        config: BatteryConfig,
    ) -> Result<Self, esp_idf_hal::sys::EspError> {
        let adc = AdcDriver::new(adc1)?;

        #[cfg(esp32)]
        let channel_config = AdcChannelConfig {
            attenuation: DB_11,
            calibration: Calibration::Line,
            ..Default::default()
        };
        #[cfg(not(esp32))]
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

        let avg_mv = (sum / valid_count) as u16;
        log::debug!(
            "Battery ADC raw: {} mV ({}/{} samples)",
            avg_mv,
            valid_count,
            num_samples
        );
        avg_mv
    }
}

impl<'a, GPIO> BatteryMonitor for EspAdcBatteryMonitor<'a, GPIO>
where
    GPIO: ADCPin<Adc = ADC1>,
{
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
