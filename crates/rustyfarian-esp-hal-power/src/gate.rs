//! GPIO-driven power rail gate on esp-hal.
//!
//! [`GpioRailGate`] implements [`PowerRailGate`] over any [`embedded_hal::digital::OutputPin`], which includes `esp_hal::gpio::Output`.
//! It is the shared `stoker::gate::GpioRailGate`; this module adds only the Heltec V3 `Vext` constructor, through the [`HeltecV3Vext`] extension trait.
//!
//! # Settle time
//!
//! The gate does not wait after `power_on`; the rail's settle time depends on the load, so the caller delays.
//!
//! # Deep sleep
//!
//! The gate is a plain GPIO driver and does not hold the pad.
//! On the Heltec V3 `Vext_Ctrl` is pulled up, so a released pad means the rail is off.
//! The output latch keeps the rail off through deep sleep without a pad hold (`bench/s3-heltec/rail-gate-ve`).

pub use stoker::gate::*;

/// Heltec WiFi LoRa 32 V3 `Vext_Ctrl` pin (GPIO36).
///
/// esp-hal exposes no pin number on `Output`; this constant names the pin for documentation and for choosing `peripherals.GPIO36`.
pub const HELTEC_V3_VEXT_GPIO: u8 = 36;

/// Heltec V3 `Vext` constructor for [`GpioRailGate`].
///
/// An extension trait, because an inherent impl on the foreign generic type is forbidden;
/// bring it into scope with `use rustyfarian_esp_hal_power::HeltecV3Vext`.
///
/// # Examples
///
/// ```rust,ignore
/// use esp_hal::gpio::{Level, Output, OutputConfig};
/// use rustyfarian_esp_hal_power::{GpioRailGate, HeltecV3Vext, PowerRailGate};
///
/// let vext = Output::new(peripherals.GPIO36, Level::High, OutputConfig::default());
/// let mut gate = GpioRailGate::heltec_v3_vext(vext).unwrap();
/// gate.power_on().unwrap();
/// ```
pub trait HeltecV3Vext: Sized {
    /// The pin the preset takes.
    type Pin;
    /// The error the preset returns.
    type Error;

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
    fn heltec_v3_vext(pin: Self::Pin) -> Result<Self, Self::Error>;
}

impl<'d> HeltecV3Vext for GpioRailGate<esp_hal::gpio::Output<'d>> {
    type Pin = esp_hal::gpio::Output<'d>;
    type Error = GpioRailGateError<core::convert::Infallible>;

    fn heltec_v3_vext(pin: esp_hal::gpio::Output<'d>) -> Result<Self, Self::Error> {
        Self::new(pin, Polarity::ActiveLow)
    }
}
