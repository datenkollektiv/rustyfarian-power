//! GPIO-driven power rail gate on esp-hal.
//!
//! [`GpioRailGate`] implements [`PowerRailGate`] over any [`embedded_hal::digital::OutputPin`], which includes `esp_hal::gpio::Output`.
//! It mirrors the ESP-IDF tier's gate; each tier defines its own generic type so `stoker` carries no HAL dependency.
//!
//! # Settle time
//!
//! The gate does not wait after `power_on`; the rail's settle time depends on the load, so the caller delays.
//!
//! # Deep sleep
//!
//! The gate is a plain GPIO driver and does not hold the pad.
//! On the Heltec V3 `Vext_Ctrl` is pulled up, so a released pad means the rail is off.
//! Whether the pad needs a hold to keep the rail off through deep sleep is an open bench question (`docs/features/power-rail-gate-v1.md`).

pub use stoker::gate::*;

use core::fmt;

use embedded_hal::digital::OutputPin;

/// Heltec WiFi LoRa 32 V3 `Vext_Ctrl` pin (GPIO36).
///
/// esp-hal exposes no pin number on `Output`; this constant names the pin for documentation and for choosing `peripherals.GPIO36`.
pub const HELTEC_V3_VEXT_GPIO: u8 = 36;

/// Output level that switches the rail on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Polarity {
    /// The rail is on while the pin is high.
    ActiveHigh,
    /// The rail is on while the pin is low.
    ActiveLow,
}

/// Error from a [`GpioRailGate`]: the underlying pin failed to change level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpioRailGateError<E> {
    /// The output pin reported an error.
    Pin(E),
}

impl<E: fmt::Debug> Error for GpioRailGateError<E> {
    fn kind(&self) -> GateErrorKind {
        GateErrorKind::Platform
    }
}

impl<E: fmt::Debug> fmt::Display for GpioRailGateError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pin(e) => write!(f, "rail gate pin error: {e:?}"),
        }
    }
}

impl<E: fmt::Debug> core::error::Error for GpioRailGateError<E> {}

/// A power rail switched by one output pin.
///
/// `is_powered` reports the state of the last successful call; the pin is never read back.
/// Both calls are idempotent and drive the pin again.
///
/// # Examples
///
/// ```rust,ignore
/// use esp_hal::gpio::{Level, Output, OutputConfig};
/// use rustyfarian_esp_hal_power::{GpioRailGate, PowerRailGate};
///
/// let vext = Output::new(peripherals.GPIO36, Level::High, OutputConfig::default());
/// let mut gate = GpioRailGate::heltec_v3_vext(vext).unwrap();
/// gate.power_on().unwrap();
/// ```
#[derive(Debug)]
pub struct GpioRailGate<P: OutputPin> {
    pin: P,
    polarity: Polarity,
    powered: bool,
}

impl<P: OutputPin> GpioRailGate<P> {
    /// Takes the pin and drives the rail off, so [`PowerRailGate::is_powered`] starts `false`
    /// and matches the pin.
    ///
    /// # Errors
    ///
    /// Returns [`GpioRailGateError::Pin`] if the pin cannot be driven.
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

impl<'d> GpioRailGate<esp_hal::gpio::Output<'d>> {
    /// Heltec WiFi LoRa 32 V3 `Vext` rail: `Vext_Ctrl` on GPIO36, active-low.
    ///
    /// The pin is pulled up and gates a P-MOSFET, so the rail is off unless the pin is driven low.
    /// The rail feeds the OLED and the `Ve` header pins.
    /// The SX1262 sits on the always-on 3V3 rail and cannot be gated.
    ///
    /// Build the output from `peripherals.GPIO36` (see [`HELTEC_V3_VEXT_GPIO`]); the gate drives it off immediately.
    ///
    /// # Errors
    ///
    /// Returns [`GpioRailGateError::Pin`] if the pin cannot be driven.
    pub fn heltec_v3_vext(
        pin: esp_hal::gpio::Output<'d>,
    ) -> Result<Self, GpioRailGateError<core::convert::Infallible>> {
        Self::new(pin, Polarity::ActiveLow)
    }
}

impl<P: OutputPin> PowerRailGate for GpioRailGate<P> {
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
