//! GPIO-driven power rail gate for the ESP-IDF tier.
//!
//! [`GpioRailGate`] is the shared `stoker::gate::GpioRailGate`, generic over any `embedded_hal` 1.x
//! `OutputPin`, so an `esp_idf_hal::gpio::PinDriver` in output mode drives a load switch directly.
//! This module adds only the Heltec V3 `Vext` constructor, through the [`HeltecV3Vext`] extension trait.
//! Settle time after switching is not modelled; the caller waits as long as its load needs.

#[cfg(esp32s3)]
use esp_idf_hal::gpio::{Gpio36, GpioError, Output, PinDriver};
pub use stoker::gate::{GpioRailGate, GpioRailGateError, Polarity};

/// GPIO number of the Heltec WiFi LoRa 32 V3 `Vext_Ctrl` pin.
pub const HELTEC_V3_VEXT_GPIO: u8 = 36;

/// Heltec V3 `Vext` constructor for [`GpioRailGate`].
///
/// An extension trait, because an inherent impl on the foreign generic type is forbidden;
/// bring it into scope with `use rustyfarian_esp_idf_power::HeltecV3Vext`.
pub trait HeltecV3Vext: Sized {
    /// The pin the preset takes.
    type Pin;
    /// The error the preset returns.
    type Error;

    /// Heltec WiFi LoRa 32 V3 `Vext` rail on GPIO36 (`Vext_Ctrl`), active LOW.
    ///
    /// `Vext_Ctrl` gates a P-MOSFET, so LOW switches the rail on.
    /// The rail feeds the OLED and the `Ve` header pins.
    /// The SX1262 sits on the always-on 3V3 rail and cannot be gated.
    ///
    /// The pad is excluded from ESP-IDF's sleep-time GPIO isolation (`gpio_sleep_sel_dis`):
    /// a released `Vext_Ctrl` turns the rail ON (measured 3.29 V on `Ve` in deep sleep), so the
    /// pad keeps driving its latched level through sleep. Do not re-enable sleep-select on it.
    ///
    /// Takes the typed `Gpio36` pad, so another pin does not compile; ESP32-S3 only
    /// (GPIO36 is input-only on the original ESP32).
    ///
    /// # Errors
    ///
    /// Returns [`GpioRailGateError::Pin`] if configuring the pin or driving the rail off fails.
    fn heltec_v3_vext(pin: Self::Pin) -> Result<Self, Self::Error>;
}

#[cfg(esp32s3)]
impl<'d> HeltecV3Vext for GpioRailGate<PinDriver<'d, Output>> {
    type Pin = Gpio36<'d>;
    type Error = GpioRailGateError<GpioError>;

    fn heltec_v3_vext(pin: Gpio36<'d>) -> Result<Self, Self::Error> {
        // SAFETY: gpio_set_level only writes the output latch of a valid pad; the typed
        // `Gpio36` proves ownership. Latching OFF (HIGH) before the driver is enabled avoids a
        // spurious ON pulse, because the latch resets LOW, which is the active level here.
        // The return code is ignored on purpose: it is ESP_ERR_INVALID_ARG only for a pad that
        // cannot drive an output, which the constant GPIO36 on the ESP32-S3 never is.
        unsafe {
            esp_idf_hal::sys::gpio_set_level(i32::from(HELTEC_V3_VEXT_GPIO), 1);
        }
        let driver =
            PinDriver::output(pin).map_err(|e| GpioRailGateError::Pin(GpioError::other(e)))?;
        // Keep the pad driven through sleep. ESP-IDF enables the per-pad sleep configuration
        // (SLP_SEL) for every GPIO at boot and `EspSleepManager` re-applies
        // `esp_sleep_config_gpio_isolate()` at sleep entry: output disabled, pulls floating. A
        // floating `Vext_Ctrl` turns the rail ON (Ve read 3.29 V in deep sleep, Heltec V3.1
        // bench 2026-11-09), so the latched OFF level must survive sleep, as it does on esp-hal.
        // SAFETY: gpio_sleep_sel_dis only clears the SLP_SEL bit of a valid pad; the typed
        // `Gpio36` proves ownership and the driver above holds the pad.
        let ret = unsafe { esp_idf_hal::sys::gpio_sleep_sel_dis(i32::from(HELTEC_V3_VEXT_GPIO)) };
        esp_idf_hal::sys::EspError::convert(ret)
            .map_err(|e| GpioRailGateError::Pin(GpioError::other(e)))?;
        Self::new(driver, Polarity::ActiveLow)
    }
}
