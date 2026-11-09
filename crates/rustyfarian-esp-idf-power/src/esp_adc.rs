//! ESP-IDF ADC battery monitor implementation.
//!
//! Reads battery voltage via an ADC pin with voltage divider compensation.
//! Any ADC1-capable GPIO pin is supported; the specific pin depends on the
//! board (e.g. GPIO1 on Heltec Wi-Fi LoRa 32 V3, GPIO35 on Adafruit ESP32 Feather V2).

use esp_idf_hal::adc::attenuation::DB_12;
use esp_idf_hal::adc::oneshot::config::AdcChannelConfig;
use esp_idf_hal::adc::oneshot::config::Calibration;
use esp_idf_hal::adc::oneshot::{AdcChannelDriver, AdcDriver};
use esp_idf_hal::adc::{AdcChannel, ADC1, ADCU1};
use esp_idf_hal::gpio::ADCPin;

use stoker::{average_mv, BatteryConfig, BatteryMonitor, BatteryStatus};

/// Battery monitor using ESP-IDF ADC oneshot driver.
///
/// Generic over `GPIO`, which must be an ADC1-capable pin.
/// In esp-idf-hal 0.46 and 0.47 the ADC1 relationship is expressed via
/// `GPIO: ADCPin` with `GPIO::AdcChannel: AdcChannel<AdcUnit = ADCU1>`.
/// Use [`BatteryConfig::heltec_v3()`] for the Heltec WiFi LoRa 32 V3 (GPIO1)
/// or construct a custom [`BatteryConfig`] for other boards.
pub struct EspAdcBatteryMonitor<'a, GPIO>
where
    GPIO: ADCPin,
    GPIO::AdcChannel: AdcChannel<AdcUnit = ADCU1>,
{
    channel: AdcChannelDriver<'a, GPIO::AdcChannel, AdcDriver<'a, ADCU1>>,
    config: BatteryConfig,
}

impl<'a, GPIO> EspAdcBatteryMonitor<'a, GPIO>
where
    GPIO: ADCPin + 'a,
    GPIO::AdcChannel: AdcChannel<AdcUnit = ADCU1>,
{
    /// Creates a new battery monitor using ADC1 and the given ADC1-capable GPIO pin.
    ///
    /// 12 dB attenuation is selected to cover the 0–2100 mV raw signal produced by a 2:1
    /// voltage divider (4200 mV battery max → 2100 mV at the ADC pin).
    /// The effective ceiling is chip-specific: 2450 mV on original ESP32, 3100 mV on ESP32-S3.
    /// Both ceilings comfortably cover the 2100 mV maximum raw signal.
    ///
    /// On the ESP32, ESP32-S2 and ESP32-C2, `Calibration::Line` is used to correct the
    /// ADC's significant non-linearity at 12 dB attenuation.
    /// Without calibration, uncalibrated readings can be hundreds of mV low,
    /// causing a healthy battery to be misclassified as `PowerSource::Unknown`.
    /// On every other chip (ESP32-S3/C3/C6), `Calibration::Curve` (eFuse curve fitting) is
    /// used so both this tier and the esp-hal tier read the same external voltage to the
    /// same millivolts — one physical `divider_ratio` can then serve both
    /// (`BatteryConfig::heltec_v3`). esp-idf-hal defines each variant only for its chips,
    /// so the `cfg` split mirrors that support matrix.
    pub fn new(
        adc1: ADC1<'a>,
        battery_pin: GPIO,
        config: BatteryConfig,
    ) -> Result<Self, esp_idf_hal::sys::EspError> {
        let adc = AdcDriver::new(adc1)?;

        #[cfg(any(esp32, esp32c2, esp32s2))]
        let channel_config = AdcChannelConfig {
            attenuation: DB_12,
            calibration: Calibration::Line,
            ..Default::default()
        };
        #[cfg(not(any(esp32, esp32c2, esp32s2)))]
        let channel_config = AdcChannelConfig {
            attenuation: DB_12,
            calibration: Calibration::Curve,
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

    /// The configuration this monitor evaluates readings with.
    pub fn config(&self) -> &BatteryConfig {
        &self.config
    }

    /// Averaged, calibrated pin millivolts before divider compensation.
    ///
    /// Takes `samples` conversions (at least one) and averages the valid ones; 0 if every
    /// conversion failed. This is the `raw_pin_mV` of the divider-ratio calibration in
    /// [`BatteryConfig::heltec_v3`]; pair it with a meter reading taken at the same moment.
    /// [`BatteryMonitor::read`] is `config().evaluate_reading(read_raw_mv())`.
    pub fn read_raw_mv(&mut self) -> u16 {
        let num_samples = self.config.samples.max(1) as usize;
        let mut valid_count = 0;

        let avg_mv = average_mv((0..num_samples).map(|_| match self.channel.read() {
            Ok(value) => {
                valid_count += 1;
                Some(value)
            }
            Err(e) => {
                log::warn!("ADC read failed: {:?}", e);
                None
            }
        }));

        if valid_count == 0 {
            log::error!("All ADC reads failed");
            return 0;
        }

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
    GPIO: ADCPin + 'a,
    GPIO::AdcChannel: AdcChannel<AdcUnit = ADCU1>,
{
    /// Read the current battery status.
    ///
    /// Takes multiple ADC samples, averages them, and delegates to
    /// [`BatteryConfig::evaluate_reading()`] for conversion.
    fn read(&mut self) -> BatteryStatus {
        let raw_mv = self.read_raw_mv();
        let status = self.config.evaluate_reading(raw_mv);
        log::debug!("Battery: {}", status);
        status
    }
}
