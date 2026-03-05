//! ESP-IDF charging monitor implementation.
//!
//! Reads the MCP73831 STAT pin and a USB VBUS detect pin to determine
//! the current charging state.

use esp_idf_hal::gpio::{Input, InputPin, Level, OutputPin, PinDriver, Pull};
use esp_idf_hal::peripheral::Peripheral;
use esp_idf_hal::sys::EspError;

use crate::{ChargingMonitor, ChargingSource, ChargingState};

/// Charging monitor for boards with an MCP73831 charge controller.
///
/// Reads two GPIO pins to resolve the four possible charging states:
///
/// | STAT (GPIO13) | VBUS (GPIO34) | [`ChargingState`]         |
/// |---------------|---------------|---------------------------|
/// | LOW           | HIGH          | `Charging { source }`     |
/// | HIGH          | HIGH          | `Full`                    |
/// | HIGH          | LOW           | `NoBattery`               |
/// | LOW           | LOW           | `Unknown` (not expected)  |
///
/// # Adafruit ESP32 Feather V2 wiring
///
/// - **STAT pin → GPIO13**: connected to the CHG LED via a series resistor.
///   The board provides an external 4.7 kΩ pull-up to 3.3 V — configure as
///   `Pull::Floating` and do not enable the internal pull-up.
///   Must be a bidirectional GPIO (`InputPin + OutputPin`) because `set_pull`
///   requires the `OutputPin` bound in esp-idf-hal.
/// - **VBUS pin → GPIO34**: a 100 kΩ + 100 kΩ voltage divider from USB 5 V.
///   GPIO34 is an input-only strapping pin on the original ESP32;
///   calling `set_pull` on it returns `ESP_ERR_INVALID_ARG` — do not call it.
///
/// # MCP73831 limitation
///
/// The STAT pin cannot distinguish charge complete from no battery when USB is
/// absent — both appear as STAT HIGH + VBUS LOW, reported here as `NoBattery`.
/// Correlate with [`crate::BatteryMonitor::read`] if you need to tell them apart:
/// a voltage ≥ 4.1 V suggests `Full`; a voltage below the minimum suggests no battery.
pub struct EspChargingMonitor<'d, STAT, VBUS>
where
    STAT: InputPin + OutputPin,
    VBUS: InputPin,
{
    stat: PinDriver<'d, STAT, Input>,
    vbus: PinDriver<'d, VBUS, Input>,
    source: ChargingSource,
}

impl<'d, STAT, VBUS> EspChargingMonitor<'d, STAT, VBUS>
where
    STAT: InputPin + OutputPin,
    VBUS: InputPin,
{
    /// Creates a new charging monitor.
    ///
    /// `stat_pin` — GPIO connected to the MCP73831 STAT output (GPIO13 on the Feather V2).
    /// Configured as input with `Pull::Floating`; relies on the external 4.7 kΩ
    /// pull-up already present on the board.
    ///
    /// `vbus_pin` — GPIO connected to a USB VBUS detect circuit (GPIO34 on the Feather V2).
    /// Configured as a plain input with no pull configuration.
    /// Do not pass GPIO34 on the original ESP32 to any `set_pull` call —
    /// it is an input-only strapping pin and will return an error.
    ///
    /// `source` — the charge source to report when STAT is LOW.
    /// Use [`ChargingSource::Usb`] for standard USB-powered chargers.
    pub fn new(
        stat_pin: impl Peripheral<P = STAT> + 'd,
        vbus_pin: impl Peripheral<P = VBUS> + 'd,
        source: ChargingSource,
    ) -> Result<Self, EspError> {
        let mut stat = PinDriver::input(stat_pin)?;
        stat.set_pull(Pull::Floating)?;
        let vbus = PinDriver::input(vbus_pin)?;
        Ok(Self { stat, vbus, source })
    }
}

impl<'d, STAT, VBUS> ChargingMonitor for EspChargingMonitor<'d, STAT, VBUS>
where
    STAT: InputPin + OutputPin,
    VBUS: InputPin,
{
    fn read_charging_state(&mut self) -> ChargingState {
        let stat_level = self.stat.get_level();
        let vbus_level = self.vbus.get_level();
        log::debug!(
            "ChargingMonitor: STAT={:?} VBUS={:?}",
            stat_level,
            vbus_level
        );

        let stat_low = stat_level == Level::Low;
        let vbus_high = vbus_level == Level::High;

        match (stat_low, vbus_high) {
            (true, true) => ChargingState::Charging {
                source: self.source,
            },
            (false, true) => ChargingState::Full,
            (false, false) => ChargingState::NoBattery,
            (true, false) => {
                log::warn!("EspChargingMonitor: STAT LOW with VBUS absent — unexpected state");
                ChargingState::Unknown
            }
        }
    }
}
