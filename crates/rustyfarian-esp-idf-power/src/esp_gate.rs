//! GPIO-driven power rail gate for the ESP-IDF tier.
//!
//! [`GpioRailGate`] implements [`stoker::PowerRailGate`] over any `embedded_hal` 1.x
//! [`OutputPin`], so an `esp_idf_hal::gpio::PinDriver` in output mode drives a load switch directly.
//! Settle time after switching is not modelled; the caller waits as long as its load needs.

use core::fmt;

use embedded_hal::digital::OutputPin;
#[cfg(esp32s3)]
use esp_idf_hal::gpio::{Gpio36, GpioError, Output, PinDriver};
use stoker::gate::{Error as GateErrorTrait, GateErrorKind};
use stoker::PowerRailGate;

/// GPIO number of the Heltec WiFi LoRa 32 V3 `Vext_Ctrl` pin.
pub const HELTEC_V3_VEXT_GPIO: u8 = 36;

/// Pin level that switches the rail on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Polarity {
    /// The rail is on while the pin is HIGH.
    ActiveHigh,
    /// The rail is on while the pin is LOW.
    ActiveLow,
}

/// Error from a [`GpioRailGate`]: the underlying pin write failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpioRailGateError<E> {
    /// The pin driver reported an error.
    Pin(E),
}

impl<E: fmt::Debug> fmt::Display for GpioRailGateError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pin(e) => write!(f, "rail gate pin error: {e:?}"),
        }
    }
}

impl<E: fmt::Debug> core::error::Error for GpioRailGateError<E> {}

impl<E: fmt::Debug> GateErrorTrait for GpioRailGateError<E> {
    fn kind(&self) -> GateErrorKind {
        GateErrorKind::Platform
    }
}

/// A power rail switched by one GPIO output.
///
/// `powered` caches the state of the last successful pin write.
/// A failed write leaves it unchanged.
pub struct GpioRailGate<P: OutputPin> {
    pin: P,
    polarity: Polarity,
    powered: bool,
}

impl<P: OutputPin> GpioRailGate<P> {
    /// Takes the pin and drives the rail OFF immediately, so [`PowerRailGate::is_powered`]
    /// starts `false` and matches the pin.
    ///
    /// # Errors
    ///
    /// Returns [`GpioRailGateError::Pin`] if the initial write fails.
    pub fn new(pin: P, polarity: Polarity) -> Result<Self, GpioRailGateError<P::Error>> {
        let mut gate = Self {
            pin,
            polarity,
            powered: false,
        };
        gate.drive(false)?;
        Ok(gate)
    }

    fn drive(&mut self, on: bool) -> Result<(), GpioRailGateError<P::Error>> {
        let high = on == (self.polarity == Polarity::ActiveHigh);
        let result = if high {
            self.pin.set_high()
        } else {
            self.pin.set_low()
        };
        result.map_err(GpioRailGateError::Pin)?;
        self.powered = on;
        Ok(())
    }
}

#[cfg(esp32s3)]
impl<'d> GpioRailGate<PinDriver<'d, Output>> {
    /// Heltec WiFi LoRa 32 V3 `Vext` rail on GPIO36 (`Vext_Ctrl`), active LOW.
    ///
    /// `Vext_Ctrl` is pulled up and gates a P-MOSFET, so LOW switches the rail on.
    /// The rail feeds the OLED and the `Ve` header pins.
    /// The SX1262 sits on the always-on 3V3 rail and cannot be gated.
    ///
    /// Takes the typed `Gpio36` pad, so another pin does not compile; ESP32-S3 only
    /// (GPIO36 is input-only on the original ESP32).
    ///
    /// # Errors
    ///
    /// Returns [`GpioRailGateError::Pin`] if configuring the pin or driving the rail off fails.
    pub fn heltec_v3_vext(pin: Gpio36<'d>) -> Result<Self, GpioRailGateError<GpioError>> {
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
        Self::new(driver, Polarity::ActiveLow)
    }
}

impl<P: OutputPin> PowerRailGate for GpioRailGate<P>
where
    P::Error: fmt::Debug,
{
    type Error = GpioRailGateError<P::Error>;

    fn power_on(&mut self) -> Result<(), Self::Error> {
        self.drive(true)
    }

    fn power_off(&mut self) -> Result<(), Self::Error> {
        self.drive(false)
    }

    fn is_powered(&self) -> bool {
        self.powered
    }
}
