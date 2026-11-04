//! Battery charging state detection.
//!
//! Provides [`ChargingMonitor`] trait, [`ChargingState`], and [`ChargingSource`] types
//! for detecting the charge trajectory of a connected LiPo battery.
//!
//! This module is hardware-independent and always compiled.
//! [`GpioChargingMonitor`] is the GPIO-backed monitor, generic over
//! `embedded_hal::digital::InputPin` (ADR-004 rung 1), so both tiers share one type.
//! The ESP-IDF wrapper (`EspChargingMonitor`) lives in the companion `rustyfarian-esp-idf-power` crate.

use core::fmt;

use embedded_hal::digital::InputPin;

/// The energy source driving the charge controller.
///
/// Carried by [`ChargingState::Charging`] to identify what is supplying charge.
/// Kept separate from [`crate::PowerSource`] because the charging source and the
/// system power source can vary independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChargingSource {
    /// Charging from USB bus power (5 V VBUS).
    Usb,
    /// Charging from a solar panel or other DC input.
    ///
    /// Reserved for future boards; the Adafruit ESP32 Feather V2 does not have
    /// a dedicated solar input rail.
    Solar,
}

impl fmt::Display for ChargingSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChargingSource::Usb => write!(f, "USB"),
            ChargingSource::Solar => write!(f, "Solar"),
        }
    }
}

/// The battery charge trajectory as reported by the charge controller.
///
/// Returned by [`ChargingMonitor::read_charging_state`].
///
/// This is orthogonal to [`crate::PowerSource`], which reports what is supplying
/// the system with energy right now.
/// `ChargingState` reports what is happening to the battery itself.
///
/// # MCP73831 STAT pin limitation
///
/// The MCP73831 STAT pin is open-drain.
/// It cannot distinguish between charge complete, no battery, and no USB power —
/// all three conditions read HIGH on the pin.
/// `EspChargingMonitor` (in `rustyfarian-esp-idf-power`) cross-references a USB VBUS detect pin to resolve
/// the `Full` vs `NoBattery` ambiguity when USB is present.
/// Distinguishing `Full` from `NoBattery` when USB is absent still requires
/// correlating with the battery voltage from [`crate::BatteryMonitor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChargingState {
    /// Charge controller is actively charging the battery.
    Charging {
        /// The energy source supplying the charge controller.
        source: ChargingSource,
    },
    /// Battery is fully charged; charge controller has terminated.
    ///
    /// Detected when USB VBUS is present and the STAT pin is HIGH.
    Full,
    /// No battery detected, or device is running on a full battery with no USB present.
    ///
    /// Detected when VBUS is absent and STAT is HIGH.
    ///
    /// # Ambiguity
    ///
    /// The MCP73831 STAT pin cannot distinguish charge-complete from no-battery when USB is absent —
    /// both conditions read HIGH on the open-drain output.
    /// This variant therefore covers two physically different states:
    /// a board with no battery attached, and a board running on a fully charged battery without USB.
    /// To tell them apart, correlate with [`crate::BatteryMonitor::read`]:
    /// a voltage ≥ 4.1 V after divider compensation suggests a full battery;
    /// a voltage below [`crate::BatteryConfig::min_voltage_mv`] suggests no battery is attached.
    NoBattery,
    /// Charging state cannot be determined.
    ///
    /// Returned when STAT is LOW but VBUS is absent (pathological; should not
    /// occur normally), or when a GPIO read fails.
    Unknown,
}

impl fmt::Display for ChargingState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChargingState::Charging { source } => write!(f, "Charging ({})", source),
            ChargingState::Full => write!(f, "Full"),
            ChargingState::NoBattery => write!(f, "No battery"),
            ChargingState::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Reads the current battery charging state from the charge controller.
///
/// Follows the same infallible convention as [`crate::BatteryMonitor::read`]:
/// hardware errors are absorbed into [`ChargingState::Unknown`] rather than
/// propagated as `Result`.
pub trait ChargingMonitor {
    /// Sample the charge controller and return the current charging state.
    fn read_charging_state(&mut self) -> ChargingState;
}

/// A no-op charging monitor for host-side unit testing.
///
/// [`ChargingMonitor::read_charging_state`] always returns the state configured
/// at construction.
/// Use the convenience constructors to target each [`ChargingState`] branch.
///
/// # Examples
///
/// ```
/// use stoker::{ChargingMonitor, NoopChargingMonitor, ChargingState, ChargingSource};
///
/// fn is_charging(monitor: &mut impl ChargingMonitor) -> bool {
///     matches!(monitor.read_charging_state(), ChargingState::Charging { .. })
/// }
///
/// assert!(is_charging(&mut NoopChargingMonitor::charging(ChargingSource::Usb)));
/// assert!(!is_charging(&mut NoopChargingMonitor::full()));
/// ```
pub struct NoopChargingMonitor {
    state: ChargingState,
}

impl NoopChargingMonitor {
    /// Creates a mock that returns the given `state` on every call.
    pub fn new(state: ChargingState) -> Self {
        Self { state }
    }

    /// Creates a mock in the `Charging` state with the given source.
    pub fn charging(source: ChargingSource) -> Self {
        Self::new(ChargingState::Charging { source })
    }

    /// Creates a mock in the `Full` state.
    pub fn full() -> Self {
        Self::new(ChargingState::Full)
    }

    /// Creates a mock in the `NoBattery` state.
    pub fn no_battery() -> Self {
        Self::new(ChargingState::NoBattery)
    }

    /// Creates a mock in the `Unknown` state.
    pub fn unknown() -> Self {
        Self::new(ChargingState::Unknown)
    }
}

impl ChargingMonitor for NoopChargingMonitor {
    fn read_charging_state(&mut self) -> ChargingState {
        self.state
    }
}

/// Classifies the STAT and VBUS pin levels of a charge controller.
///
/// Pure core of the GPIO charging monitors.
/// `stat_low` is `true` when the open-drain STAT pin reads low, `vbus_high` when VBUS is present.
/// STAT low with VBUS absent is the pathological combination and yields
/// [`ChargingState::Unknown`]; the caller may log it.
#[must_use]
pub fn classify_charging(stat_low: bool, vbus_high: bool, source: ChargingSource) -> ChargingState {
    match (stat_low, vbus_high) {
        (true, true) => ChargingState::Charging { source },
        (false, true) => ChargingState::Full,
        (false, false) => ChargingState::NoBattery,
        (true, false) => ChargingState::Unknown,
    }
}

/// GPIO-backed [`ChargingMonitor`] for an MCP73831-style charge controller.
///
/// Reads a STAT pin (low means charging) and a VBUS-detect pin (high means present),
/// then classifies them with [`classify_charging`].
/// A pin read error yields [`ChargingState::Unknown`], because the trait is infallible.
///
/// Pin configuration (pull, direction) is the caller's job; this type only reads.
///
/// | STAT | VBUS | [`ChargingState`]         |
/// |------|------|---------------------------|
/// | LOW  | HIGH | `Charging { source }`     |
/// | HIGH | HIGH | `Full`                    |
/// | HIGH | LOW  | `NoBattery`               |
/// | LOW  | LOW  | `Unknown` (not expected)  |
pub struct GpioChargingMonitor<STAT: InputPin, VBUS: InputPin> {
    stat: STAT,
    vbus: VBUS,
    source: ChargingSource,
}

impl<STAT: InputPin, VBUS: InputPin> GpioChargingMonitor<STAT, VBUS> {
    /// Creates a monitor over two configured input pins.
    ///
    /// `source` is the charge source reported when STAT is low.
    pub fn new(stat: STAT, vbus: VBUS, source: ChargingSource) -> Self {
        Self { stat, vbus, source }
    }
}

impl<STAT: InputPin, VBUS: InputPin> ChargingMonitor for GpioChargingMonitor<STAT, VBUS> {
    fn read_charging_state(&mut self) -> ChargingState {
        match (self.stat.is_low(), self.vbus.is_high()) {
            (Ok(stat_low), Ok(vbus_high)) => classify_charging(stat_low, vbus_high, self.source),
            _ => ChargingState::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use embedded_hal::digital::{ErrorType, InputPin};

    struct MockPin(Result<bool, ()>);

    #[derive(Debug)]
    struct MockErr;

    impl embedded_hal::digital::Error for MockErr {
        fn kind(&self) -> embedded_hal::digital::ErrorKind {
            embedded_hal::digital::ErrorKind::Other
        }
    }

    impl ErrorType for MockPin {
        type Error = MockErr;
    }

    impl InputPin for MockPin {
        fn is_high(&mut self) -> Result<bool, MockErr> {
            self.0.map_err(|_| MockErr)
        }
        fn is_low(&mut self) -> Result<bool, MockErr> {
            self.0.map(|h| !h).map_err(|_| MockErr)
        }
    }

    fn read(stat: Result<bool, ()>, vbus: Result<bool, ()>) -> ChargingState {
        GpioChargingMonitor::new(MockPin(stat), MockPin(vbus), ChargingSource::Usb)
            .read_charging_state()
    }

    #[test]
    fn gpio_monitor_four_combinations() {
        assert_eq!(
            read(Ok(false), Ok(true)),
            ChargingState::Charging {
                source: ChargingSource::Usb
            }
        );
        assert_eq!(read(Ok(true), Ok(true)), ChargingState::Full);
        assert_eq!(read(Ok(true), Ok(false)), ChargingState::NoBattery);
        assert_eq!(read(Ok(false), Ok(false)), ChargingState::Unknown);
    }

    #[test]
    fn gpio_monitor_pin_error_is_unknown() {
        assert_eq!(read(Err(()), Ok(true)), ChargingState::Unknown);
        assert_eq!(read(Ok(false), Err(())), ChargingState::Unknown);
    }

    #[test]
    fn classify_charging_truth_table() {
        let usb = ChargingSource::Usb;
        assert_eq!(
            classify_charging(true, true, usb),
            ChargingState::Charging { source: usb }
        );
        assert_eq!(
            classify_charging(true, true, ChargingSource::Solar),
            ChargingState::Charging {
                source: ChargingSource::Solar
            }
        );
        assert_eq!(classify_charging(false, true, usb), ChargingState::Full);
        assert_eq!(
            classify_charging(false, false, usb),
            ChargingState::NoBattery
        );
        assert_eq!(classify_charging(true, false, usb), ChargingState::Unknown);
    }

    #[test]
    fn display_charging_usb() {
        assert_eq!(
            format!(
                "{}",
                ChargingState::Charging {
                    source: ChargingSource::Usb
                }
            ),
            "Charging (USB)"
        );
    }

    #[test]
    fn display_charging_solar() {
        assert_eq!(
            format!(
                "{}",
                ChargingState::Charging {
                    source: ChargingSource::Solar
                }
            ),
            "Charging (Solar)"
        );
    }

    #[test]
    fn display_full() {
        assert_eq!(format!("{}", ChargingState::Full), "Full");
    }

    #[test]
    fn display_no_battery() {
        assert_eq!(format!("{}", ChargingState::NoBattery), "No battery");
    }

    #[test]
    fn display_unknown() {
        assert_eq!(format!("{}", ChargingState::Unknown), "Unknown");
    }

    #[test]
    fn noop_charging_returns_charging() {
        let mut mon = NoopChargingMonitor::charging(ChargingSource::Usb);
        assert_eq!(
            mon.read_charging_state(),
            ChargingState::Charging {
                source: ChargingSource::Usb
            }
        );
    }

    #[test]
    fn noop_full_returns_full() {
        let mut mon = NoopChargingMonitor::full();
        assert_eq!(mon.read_charging_state(), ChargingState::Full);
    }

    #[test]
    fn noop_no_battery_returns_no_battery() {
        let mut mon = NoopChargingMonitor::no_battery();
        assert_eq!(mon.read_charging_state(), ChargingState::NoBattery);
    }

    #[test]
    fn noop_unknown_returns_unknown() {
        let mut mon = NoopChargingMonitor::unknown();
        assert_eq!(mon.read_charging_state(), ChargingState::Unknown);
    }
}
