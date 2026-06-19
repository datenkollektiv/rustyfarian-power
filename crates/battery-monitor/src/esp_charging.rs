//! ESP-IDF charging monitor implementation.
//!
//! Reads the MCP73831 STAT pin and a USB VBUS detect pin to determine
//! the current charging state.

use esp_idf_hal::gpio::{Input, InputPin, Level, OutputPin, PinDriver, Pull};
use esp_idf_hal::sys::EspError;

use crate::{ChargingMonitor, ChargingSource, ChargingState};

/// Charging monitor for boards with an MCP73831 charge controller.
///
/// Reads a STAT pin and a USB-VBUS-detect pin to resolve the four charging states:
///
/// | STAT | VBUS | [`ChargingState`]         |
/// |------|------|---------------------------|
/// | LOW  | HIGH | `Charging { source }`     |
/// | HIGH | HIGH | `Full`                    |
/// | HIGH | LOW  | `NoBattery`               |
/// | LOW  | LOW  | `Unknown` (not expected)  |
///
/// # Required wiring
///
/// This monitor only works on a board that actually routes both signals to readable GPIOs:
///
/// - **STAT pin**: the MCP73831 STAT output (a tri-state logic output — LOW while charging,
///   Hi-Z otherwise) brought to a GPIO with an external pull-up to 3.3 V. Configured as
///   `Pull::Floating` so the internal pull-up is not enabled. `STAT` must satisfy
///   `InputPin + OutputPin` because esp-idf-hal requires `OutputPin` for the
///   `PinDriver::input` constructor on bidirectional GPIOs.
/// - **VBUS pin**: a USB 5 V presence signal divided into the 0–3.3 V range and brought to a
///   GPIO. Configured as `Pull::Floating` so no pull resistor is requested (this allows an
///   input-only pin, such as GPIO34–39 on the original ESP32, to be used).
///
/// **Not usable on a stock Adafruit ESP32 Feather V2.** That board exposes neither signal on
/// a GPIO — the MCP73831 STAT drives only the on-board CHG LED, GPIO13 is the user LED, and
/// there is no VBUS-detect divider. This was confirmed on hardware (see the
/// `idf_esp32_chargeprobe` example): GPIO13/GPIO34 read floating, not STAT/VBUS levels. On
/// that board, infer charging indirectly from the battery voltage rising over time.
///
/// # MCP73831 limitation
///
/// The STAT pin cannot distinguish charge complete from no battery when USB is
/// absent — both appear as STAT HIGH + VBUS LOW, reported here as `NoBattery`.
/// Correlate with [`crate::BatteryMonitor::read`] if you need to tell them apart:
/// a voltage ≥ 4.1 V suggests `Full`; a voltage below the minimum suggests no battery.
pub struct EspChargingMonitor<'d> {
    stat: PinDriver<'d, Input>,
    vbus: PinDriver<'d, Input>,
    source: ChargingSource,
}

impl<'d> EspChargingMonitor<'d> {
    /// Creates a new charging monitor.
    ///
    /// `stat_pin` — GPIO connected to the MCP73831 STAT output (GPIO13 on the Feather V2).
    /// Configured as input with `Pull::Floating`; relies on the external 4.7 kΩ
    /// pull-up already present on the board.
    ///
    /// `vbus_pin` — GPIO connected to a USB VBUS detect circuit (GPIO34 on the Feather V2).
    /// Configured as input with `Pull::Floating`.
    /// GPIO34 on the original ESP32 is an input-only strapping pin; `Pull::Floating`
    /// avoids configuring a pull resistor, which would fail on that pin.
    ///
    /// `source` — the charge source to report when STAT is LOW.
    /// Use [`ChargingSource::Usb`] for standard USB-powered chargers.
    pub fn new<STAT, VBUS>(
        stat_pin: STAT,
        vbus_pin: VBUS,
        source: ChargingSource,
    ) -> Result<Self, EspError>
    where
        STAT: InputPin + OutputPin + 'd,
        VBUS: InputPin + 'd,
    {
        let stat = PinDriver::input(stat_pin, Pull::Floating)?;
        // VBUS is input-only on some targets (e.g. GPIO34 on original ESP32).
        // Pull::Floating avoids calling set_pull on a pin that does not support it.
        let vbus = PinDriver::input(vbus_pin, Pull::Floating)?;
        Ok(Self { stat, vbus, source })
    }
}

impl ChargingMonitor for EspChargingMonitor<'_> {
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
