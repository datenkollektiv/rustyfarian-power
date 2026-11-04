//! ADC battery monitor on esp-hal.
//!
//! [`EspHalAdcBatteryMonitor`] reads the battery voltage through an ADC1 pin and a resistive divider.
//! It follows the same flow as the ESP-IDF tier's `EspAdcBatteryMonitor`:
//! average `BatteryConfig::samples` calibrated millivolt readings, then let [`BatteryConfig::evaluate_reading`] undo the divider and classify the result.
//!
//! The pin is configured for 11 dB attenuation (esp-hal's name for the ESP-IDF `DB_12` setting).
//! That covers the 0-2100 mV signal of a 2:1 divider on a 4200 mV cell; the usable ceiling is chip-specific (about 2450 mV on the ESP32, about 3100 mV on the S3).
//!
//! Calibration is chosen per chip: line fitting on the ESP32, curve fitting on the C3, C6 and S3.
//! Both read the factory eFuse data, so `read_oneshot` returns millivolts rather than raw counts.
//! Without line fitting the ESP32 reads hundreds of millivolts low at 11 dB, which would misclassify a healthy cell.
//!
//! Only ADC1 is used.
//! esp-hal does not tie a pin type to an ADC unit: its `AdcChannel` only carries the channel number, so an ADC2 pad would silently read the ADC1 channel with the same number.
//! The sealed [`Adc1Pin`] marker closes that gap: only the ADC1 pads of the chip are accepted (GPIO1 on the Heltec V3, GPIO35 on the Feather V2).
//! ADC2 is also unusable on the ESP32 while Wi-Fi or Bluetooth is active.

use esp_hal::analog::adc::{Adc, AdcCalScheme, AdcChannel, AdcConfig, AdcPin, Attenuation};
use esp_hal::gpio::AnalogPin;
use esp_hal::peripherals::ADC1;
use esp_hal::Blocking;
use stoker::{average_mv, BatteryConfig, BatteryMonitor, BatteryStatus};

#[cfg(not(feature = "esp32"))]
use esp_hal::analog::adc::AdcCalCurve;
#[cfg(feature = "esp32")]
use esp_hal::analog::adc::AdcCalLine;

mod sealed {
    pub trait Sealed {}
}

/// Marker for the GPIO pads wired to ADC1 on this chip.
///
/// Sealed: implemented here for the ADC1 pads only, from the pad tables of the chip (ESP32
/// GPIO32-39, S3 GPIO1-10, C3 GPIO0-4, C6 GPIO0-6).
pub trait Adc1Pin: sealed::Sealed {}

macro_rules! adc1_pins {
    ($($gpio:ident),+) => {
        $(
            impl sealed::Sealed for esp_hal::peripherals::$gpio<'_> {}
            impl Adc1Pin for esp_hal::peripherals::$gpio<'_> {}
        )+
    };
}

#[cfg(feature = "esp32")]
adc1_pins!(GPIO32, GPIO33, GPIO34, GPIO35, GPIO36, GPIO37, GPIO38, GPIO39);
#[cfg(feature = "esp32s3")]
adc1_pins!(GPIO1, GPIO2, GPIO3, GPIO4, GPIO5, GPIO6, GPIO7, GPIO8, GPIO9, GPIO10);
#[cfg(feature = "esp32c3")]
adc1_pins!(GPIO0, GPIO1, GPIO2, GPIO3, GPIO4);
#[cfg(feature = "esp32c6")]
adc1_pins!(GPIO0, GPIO1, GPIO2, GPIO3, GPIO4, GPIO5, GPIO6);

/// Calibration scheme of this chip: line fitting on the ESP32, curve fitting elsewhere.
#[cfg(feature = "esp32")]
type Cal<'d> = AdcCalLine<ADC1<'d>>;
/// Calibration scheme of this chip: line fitting on the ESP32, curve fitting elsewhere.
#[cfg(not(feature = "esp32"))]
type Cal<'d> = AdcCalCurve<ADC1<'d>>;

/// Battery monitor over an ADC1 pin, generic over the GPIO.
///
/// Use [`BatteryConfig::heltec_v3`] for the Heltec WiFi LoRa 32 V3 (GPIO1), [`BatteryConfig::adafruit_feather_v2`] for the Feather V2 (GPIO35), or a custom [`BatteryConfig`].
pub struct EspHalAdcBatteryMonitor<'d, PIN> {
    adc: Adc<'d, ADC1<'d>, Blocking>,
    pin: AdcPin<PIN, ADC1<'d>, Cal<'d>>,
    config: BatteryConfig,
}

impl<'d, PIN> EspHalAdcBatteryMonitor<'d, PIN>
where
    PIN: AdcChannel + AnalogPin + Adc1Pin,
    Cal<'d>: AdcCalScheme<ADC1<'d>>,
{
    /// Creates a monitor on ADC1 and the given ADC1-capable GPIO pin.
    ///
    /// Takes ownership of the ADC1 peripheral and the pin, and switches the pin to analog mode.
    pub fn new(adc1: ADC1<'d>, battery_pin: PIN, config: BatteryConfig) -> Self {
        let mut adc_config = AdcConfig::new();
        let pin = adc_config.enable_pin_with_cal::<PIN, Cal<'d>>(battery_pin, Attenuation::_11dB);
        let adc = Adc::new(adc1, adc_config);
        Self { adc, pin, config }
    }

    /// One blocking conversion in millivolts; `None` if the ADC reports an error.
    fn read_one_mv(&mut self) -> Option<u16> {
        nb::block!(self.adc.read_oneshot(&mut self.pin)).ok()
    }
}

impl<'d, PIN> BatteryMonitor for EspHalAdcBatteryMonitor<'d, PIN>
where
    PIN: AdcChannel + AnalogPin + Adc1Pin,
    Cal<'d>: AdcCalScheme<ADC1<'d>>,
{
    /// Takes `samples` conversions (at least one), averages the valid ones, and delegates to [`BatteryConfig::evaluate_reading`].
    ///
    /// If every conversion fails the averaged reading is 0 mV, which the config reports as no battery.
    fn read(&mut self) -> BatteryStatus {
        let samples = self.config.samples.max(1);
        let raw_mv = average_mv((0..samples).map(|_| self.read_one_mv()));
        self.config.evaluate_reading(raw_mv)
    }
}
