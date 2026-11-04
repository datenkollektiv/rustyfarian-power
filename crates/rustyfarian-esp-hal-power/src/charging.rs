//! GPIO-driven charging monitor on esp-hal.
//!
//! [`GpioChargingMonitor`] implements [`ChargingMonitor`] over any [`embedded_hal::digital::InputPin`], which includes `esp_hal::gpio::Input`.
//! It is the shared `stoker::charging::GpioChargingMonitor`; this module adds no logic.
//!
//! Configure the pins before constructing the monitor.
//! STAT needs an external pull-up (open-drain output, low while charging); VBUS is a divided 5 V presence signal.
//! Both boards in this repo (Feather V2, Heltec V3) expose neither signal on a GPIO, so this needs a board that routes them.

pub use stoker::charging::GpioChargingMonitor;

/// Charging monitor over two configured `esp_hal::gpio::Input` pins.
pub type EspHalChargingMonitor<'d> =
    GpioChargingMonitor<esp_hal::gpio::Input<'d>, esp_hal::gpio::Input<'d>>;
