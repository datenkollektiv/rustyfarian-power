//! Power rail gate trait, host-side mock and sleep coordinator.
//!
//! # Architecture
//!
//! A [`PowerRailGate`] switches one board power rail (for example the Heltec V3 `Vext` rail)
//! on and off.
//! It is a rail primitive: it carries no radio or network types, and it does not model settle time.
//! Load-dependent settling is the consumer's job: delay after [`PowerRailGate::power_on`].
//!
//! [`GatedSleep`] pairs a gate with a [`SleepManager`] so the rail is switched off before sleep.
//! Deep sleep never returns on real hardware, so "on after wake" is not a call in this module:
//! the consumer calls [`PowerRailGate::power_on`] at boot.
//! The coordinator restores the rail only when the sleep call returns an error.
//!
//! The GPIO-backed gates (`GpioRailGate`) live in the tier crates, generic over
//! `embedded_hal::digital::OutputPin`, so `stoker` keeps `critical-section` as its only dependency.
//!
//! ```
//! use stoker::{
//!     GatedSleep, NoopPowerRailGate, NoopSleepManager, PowerRailGate, SleepManager, WakeSource,
//! };
//!
//! let mut coordinator = GatedSleep::new(NoopPowerRailGate::on(), NoopSleepManager::default());
//! assert!(coordinator.gate_mut().is_powered());
//!
//! // The rail is switched off before the sleep call.
//! coordinator
//!     .sleep(&[WakeSource::Timer { duration_ms: 60_000 }])
//!     .unwrap();
//! assert!(!coordinator.gate_mut().is_powered());
//!
//! // A failed sleep puts the rail back and reports the sleep error.
//! let mut coordinator = GatedSleep::new(NoopPowerRailGate::on(), NoopSleepManager::default());
//! assert!(coordinator.sleep(&[]).is_err());
//! assert!(coordinator.gate_mut().is_powered());
//!
//! // After wake the consumer powers the rail on again.
//! let (mut gate, _sleep) = coordinator.into_parts();
//! gate.power_on().unwrap();
//! ```
//!
//! # Errors
//!
//! [`PowerRailGate::Error`] is bounded by the [`Error`] trait, which exposes a HAL-neutral
//! [`GateErrorKind`] through [`Error::kind`], mirroring [`crate::sleep::Error`].
//! [`GatedSleepError`] tells whether the gate or the sleep manager failed.

use crate::sleep::{Error as SleepErrorTrait, SleepErrorKind, SleepManager, WakeSource};

/// Error type of a [`PowerRailGate`] implementation.
///
/// Mirrors [`crate::sleep::Error`]: [`core::fmt::Debug`] plus a HAL-neutral [`kind`](Error::kind).
/// It deliberately does **not** require [`core::error::Error`].
pub trait Error: core::fmt::Debug {
    /// Returns the HAL-neutral classification of this error.
    fn kind(&self) -> GateErrorKind;
}

/// HAL-neutral classification of a [`PowerRailGate`] error.
///
/// `#[non_exhaustive]`: downstream `match`es must include a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum GateErrorKind {
    /// A HAL or SDK failure occurred while driving the rail (for example a GPIO write failed).
    Platform,
    /// Catch-all for variants added in the future.
    Other,
}

/// Switches one board power rail on and off.
///
/// Both transitions are idempotent: switching to the state the gate believes it is already in
/// returns `Ok(())` and drives the pin again, so a stale cached state cannot leave the rail wrong.
///
/// The gate models no settle time; delay after [`power_on`](Self::power_on) as the load requires.
pub trait PowerRailGate {
    /// Error returned when the rail cannot be driven.
    type Error: Error;

    /// Switches the rail on.
    ///
    /// # Errors
    ///
    /// Returns an error if the rail cannot be driven; [`is_powered`](Self::is_powered) then keeps
    /// the state of the last successful call.
    #[must_use = "a failed power_on leaves the rail off — handle the error"]
    fn power_on(&mut self) -> Result<(), Self::Error>;

    /// Switches the rail off.
    ///
    /// # Errors
    ///
    /// Returns an error if the rail cannot be driven; [`is_powered`](Self::is_powered) then keeps
    /// the state of the last successful call.
    #[must_use = "a failed power_off leaves the rail on and drains the battery — handle the error"]
    fn power_off(&mut self) -> Result<(), Self::Error>;

    /// Returns the state set by the last successful [`power_on`](Self::power_on) or
    /// [`power_off`](Self::power_off).
    ///
    /// Cached and infallible: `OutputPin` cannot read itself back on every HAL.
    fn is_powered(&self) -> bool;
}

/// Errors returned by [`NoopPowerRailGate`].
///
/// Implements [`Error`], so [`Error::kind`] classifies every variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum GateError {
    /// The mock was told to refuse the next transition.
    Refused,
}

impl core::fmt::Display for GateError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Refused => write!(f, "power rail gate refused the transition"),
        }
    }
}

impl core::error::Error for GateError {}

impl Error for GateError {
    fn kind(&self) -> GateErrorKind {
        match self {
            Self::Refused => GateErrorKind::Platform,
        }
    }
}

/// A host-side mock [`PowerRailGate`] for unit testing.
///
/// Tracks the powered state and counts calls, including failed ones.
/// [`fail_next_on`](Self::fail_next_on) and [`fail_next_off`](Self::fail_next_off) make exactly
/// one following call return [`GateError::Refused`] and leave the powered state unchanged.
///
/// # Examples
///
/// ```
/// use stoker::{GateError, NoopPowerRailGate, PowerRailGate};
///
/// let mut gate = NoopPowerRailGate::off();
/// assert!(!gate.is_powered());
///
/// gate.fail_next_on();
/// assert_eq!(gate.power_on(), Err(GateError::Refused));
/// assert!(!gate.is_powered());
///
/// gate.power_on().unwrap();
/// assert!(gate.is_powered());
/// assert_eq!(gate.on_calls(), 2);
/// ```
#[derive(Debug, Clone, Default)]
pub struct NoopPowerRailGate {
    powered: bool,
    fail_on: bool,
    fail_off: bool,
    on_calls: u32,
    off_calls: u32,
}

impl NoopPowerRailGate {
    /// Creates a mock whose rail starts off (same as [`Default`]).
    pub fn off() -> Self {
        Self::default()
    }

    /// Creates a mock whose rail starts on.
    pub fn on() -> Self {
        Self {
            powered: true,
            ..Self::default()
        }
    }

    /// Makes the next [`PowerRailGate::power_on`] return [`GateError::Refused`].
    pub fn fail_next_on(&mut self) {
        self.fail_on = true;
    }

    /// Makes the next [`PowerRailGate::power_off`] return [`GateError::Refused`].
    pub fn fail_next_off(&mut self) {
        self.fail_off = true;
    }

    /// Number of [`PowerRailGate::power_on`] calls, including failed ones.
    pub fn on_calls(&self) -> u32 {
        self.on_calls
    }

    /// Number of [`PowerRailGate::power_off`] calls, including failed ones.
    pub fn off_calls(&self) -> u32 {
        self.off_calls
    }
}

impl PowerRailGate for NoopPowerRailGate {
    type Error = GateError;

    fn power_on(&mut self) -> Result<(), GateError> {
        self.on_calls += 1;
        if core::mem::take(&mut self.fail_on) {
            return Err(GateError::Refused);
        }
        self.powered = true;
        Ok(())
    }

    fn power_off(&mut self) -> Result<(), GateError> {
        self.off_calls += 1;
        if core::mem::take(&mut self.fail_off) {
            return Err(GateError::Refused);
        }
        self.powered = false;
        Ok(())
    }

    fn is_powered(&self) -> bool {
        self.powered
    }
}

/// Error of [`GatedSleep`]: which half failed.
///
/// Implements [`crate::sleep::Error`]: a gate failure is [`SleepErrorKind::Platform`], a sleep
/// failure keeps the kind of the wrapped error (so [`SleepErrorKind::Rejected`] stays visible).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatedSleepError<G, S> {
    /// Switching the rail off failed; the device did not sleep.
    Gate(G),
    /// The sleep manager failed or rejected; the rail was restored to its pre-call state (best effort).
    Sleep(S),
}

impl<G: core::fmt::Debug, S: core::fmt::Debug> core::fmt::Display for GatedSleepError<G, S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Gate(e) => write!(f, "power rail gate failed before sleep: {e:?}"),
            Self::Sleep(e) => write!(f, "sleep failed: {e:?}"),
        }
    }
}

impl<G: core::fmt::Debug, S: core::fmt::Debug> core::error::Error for GatedSleepError<G, S> {}

impl<G: Error, S: SleepErrorTrait> SleepErrorTrait for GatedSleepError<G, S> {
    fn kind(&self) -> SleepErrorKind {
        match self {
            Self::Gate(_) => SleepErrorKind::Platform,
            Self::Sleep(e) => e.kind(),
        }
    }
}

/// Switches a rail off before every sleep call.
///
/// Owns the gate and the sleep manager by value; [`into_parts`](Self::into_parts) returns them.
/// Implements [`SleepManager`], so it drops in where the plain manager was used.
///
/// - The rail is powered off first; a gate failure returns [`GatedSleepError::Gate`] without sleeping.
/// - If the sleep call then returns an error (including a [`SleepErrorKind::Rejected`] result
///   from `try_sleep`), the rail is restored to its pre-call state on a best-effort basis (powered
///   on only if it was on before the call) and the sleep error is returned; a failure of that
///   restore is ignored.
/// - On hardware deep sleep never returns, so powering the rail on after wake is the consumer's
///   call to [`PowerRailGate::power_on`] at boot.
pub struct GatedSleep<G: PowerRailGate, S: SleepManager> {
    gate: G,
    sleep: S,
}

impl<G: PowerRailGate, S: SleepManager> GatedSleep<G, S> {
    /// Creates a coordinator from a gate and a sleep manager.
    pub fn new(gate: G, sleep: S) -> Self {
        Self { gate, sleep }
    }

    /// Splits the coordinator back into the gate and the sleep manager.
    pub fn into_parts(self) -> (G, S) {
        (self.gate, self.sleep)
    }

    /// Mutable access to the gate, for example to call `power_on` after wake.
    pub fn gate_mut(&mut self) -> &mut G {
        &mut self.gate
    }

    /// Mutable access to the sleep manager.
    pub fn sleep_mut(&mut self) -> &mut S {
        &mut self.sleep
    }

    fn restore_on_error(
        &mut self,
        was_on: bool,
        result: Result<(), S::Error>,
    ) -> Result<(), GatedSleepError<G::Error, S::Error>> {
        match result {
            Ok(()) => Ok(()),
            Err(e) => {
                if was_on {
                    let _ = self.gate.power_on();
                }
                Err(GatedSleepError::Sleep(e))
            }
        }
    }
}

impl<G: PowerRailGate, S: SleepManager> SleepManager for GatedSleep<G, S> {
    type Error = GatedSleepError<G::Error, S::Error>;

    /// Powers the rail off, then calls [`SleepManager::sleep`].
    ///
    /// # Errors
    ///
    /// - [`GatedSleepError::Gate`] if the rail could not be switched off; the device did not sleep.
    /// - [`GatedSleepError::Sleep`] if the sleep manager failed; the rail was restored to its pre-call state.
    ///   A failed restore is swallowed because the sleep error wins; check
    ///   [`gate_mut`](Self::gate_mut)`().is_powered()` if the rail state matters afterwards.
    fn sleep(&mut self, sources: &[WakeSource]) -> Result<(), Self::Error> {
        let was_on = self.gate.is_powered();
        self.gate.power_off().map_err(GatedSleepError::Gate)?;
        let result = self.sleep.sleep(sources);
        self.restore_on_error(was_on, result)
    }

    /// Powers the rail off, then calls [`SleepManager::try_sleep`].
    ///
    /// # Errors
    ///
    /// As [`sleep`](Self::sleep); a rejected sleep is a [`GatedSleepError::Sleep`] whose
    /// [`kind`](SleepErrorTrait::kind) is [`SleepErrorKind::Rejected`], and the rail is restored.
    /// A failed restore is swallowed here too; the gate's `is_powered` tells.
    fn try_sleep(&mut self, sources: &[WakeSource]) -> Result<(), Self::Error> {
        let was_on = self.gate.is_powered();
        self.gate.power_off().map_err(GatedSleepError::Gate)?;
        let result = self.sleep.try_sleep(sources);
        self.restore_on_error(was_on, result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sleep::{NoopSleepManager, SleepError};
    use std::string::ToString;

    const TIMER: [WakeSource; 1] = [WakeSource::Timer { duration_ms: 1000 }];

    #[test]
    fn default_gate_is_off() {
        let gate = NoopPowerRailGate::default();
        assert!(!gate.is_powered());
        assert!(NoopPowerRailGate::on().is_powered());
    }

    #[test]
    fn double_power_on_is_ok_and_drives_again() {
        let mut gate = NoopPowerRailGate::off();
        gate.power_on().unwrap();
        gate.power_on().unwrap();
        assert!(gate.is_powered());
        assert_eq!(gate.on_calls(), 2);
    }

    #[test]
    fn power_off_while_off_is_ok() {
        let mut gate = NoopPowerRailGate::off();
        gate.power_off().unwrap();
        assert!(!gate.is_powered());
        assert_eq!(gate.off_calls(), 1);
    }

    #[test]
    fn injected_failure_leaves_state_unchanged_and_is_one_shot() {
        let mut gate = NoopPowerRailGate::off();
        gate.fail_next_on();
        assert_eq!(gate.power_on(), Err(GateError::Refused));
        assert!(!gate.is_powered());
        gate.power_on().unwrap();
        assert!(gate.is_powered());

        gate.fail_next_off();
        assert_eq!(gate.power_off(), Err(GateError::Refused));
        assert!(gate.is_powered());
        gate.power_off().unwrap();
        assert!(!gate.is_powered());
    }

    #[test]
    fn gate_error_kind_and_display() {
        assert_eq!(GateError::Refused.kind(), GateErrorKind::Platform);
        assert_eq!(
            GateError::Refused.to_string(),
            "power rail gate refused the transition"
        );
        assert_ne!(GateErrorKind::Platform, GateErrorKind::Other);
    }

    #[test]
    fn sleep_powers_off_first() {
        let mut c = GatedSleep::new(NoopPowerRailGate::on(), NoopSleepManager::default());
        c.sleep(&TIMER).unwrap();
        assert!(!c.gate_mut().is_powered());
        assert_eq!(c.gate_mut().off_calls(), 1);
        assert_eq!(c.gate_mut().on_calls(), 0);
    }

    #[test]
    fn try_sleep_powers_off_first() {
        let mut c = GatedSleep::new(NoopPowerRailGate::on(), NoopSleepManager::default());
        c.try_sleep(&TIMER).unwrap();
        assert!(!c.gate_mut().is_powered());
    }

    #[test]
    fn sleep_error_restores_rail_and_sleep_error_wins() {
        let mut c = GatedSleep::new(NoopPowerRailGate::on(), NoopSleepManager::default());
        let err = c.sleep(&[]).unwrap_err();
        assert_eq!(err, GatedSleepError::Sleep(SleepError::NoWakeSource));
        assert!(c.gate_mut().is_powered());
        assert_eq!(c.gate_mut().off_calls(), 1);
        assert_eq!(c.gate_mut().on_calls(), 1);
    }

    #[test]
    fn sleep_error_keeps_rail_off_when_it_was_off() {
        let mut c = GatedSleep::new(NoopPowerRailGate::off(), NoopSleepManager::default());
        assert!(c.sleep(&[]).is_err());
        assert!(!c.gate_mut().is_powered());
        assert_eq!(c.gate_mut().on_calls(), 0);

        let mut c = GatedSleep::new(NoopPowerRailGate::off(), NoopSleepManager::rejecting());
        assert!(c.try_sleep(&TIMER).is_err());
        assert!(!c.gate_mut().is_powered());
        assert_eq!(c.gate_mut().on_calls(), 0);
    }

    #[test]
    fn gated_sleep_error_display() {
        let e: GatedSleepError<GateError, SleepError> = GatedSleepError::Gate(GateError::Refused);
        assert!(e.to_string().contains("Refused"));
        let _: &dyn core::error::Error = &e;
    }

    #[test]
    fn failed_restore_is_ignored() {
        let mut gate = NoopPowerRailGate::on();
        gate.fail_next_on();
        let mut c = GatedSleep::new(gate, NoopSleepManager::default());
        assert_eq!(
            c.sleep(&[]),
            Err(GatedSleepError::Sleep(SleepError::NoWakeSource))
        );
        assert!(!c.gate_mut().is_powered());
    }

    #[test]
    fn rejected_try_sleep_restores_rail() {
        let mut c = GatedSleep::new(NoopPowerRailGate::on(), NoopSleepManager::rejecting());
        let err = c.try_sleep(&TIMER).unwrap_err();
        assert_eq!(err, GatedSleepError::Sleep(SleepError::Rejected));
        assert_eq!(err.kind(), SleepErrorKind::Rejected);
        assert!(c.gate_mut().is_powered());
    }

    #[test]
    fn gate_failure_does_not_sleep() {
        let mut gate = NoopPowerRailGate::on();
        gate.fail_next_off();
        let mut c = GatedSleep::new(gate, NoopSleepManager::rejecting());
        // A rejecting manager would report Rejected if it had been reached.
        let err = c.try_sleep(&TIMER).unwrap_err();
        assert_eq!(err, GatedSleepError::Gate(GateError::Refused));
        assert_eq!(err.kind(), SleepErrorKind::Platform);
        assert!(c.gate_mut().is_powered());
        assert_eq!(c.gate_mut().on_calls(), 0);

        let mut gate = NoopPowerRailGate::on();
        gate.fail_next_off();
        let mut c = GatedSleep::new(gate, NoopSleepManager::default());
        assert_eq!(
            c.sleep(&TIMER),
            Err(GatedSleepError::Gate(GateError::Refused))
        );
    }

    #[test]
    fn kind_forwards_sleep_kind() {
        let e: GatedSleepError<GateError, SleepError> =
            GatedSleepError::Sleep(SleepError::NoWakeSource);
        assert_eq!(e.kind(), SleepErrorKind::InvalidConfig);
    }

    #[test]
    fn into_parts_returns_both() {
        let mut c = GatedSleep::new(NoopPowerRailGate::on(), NoopSleepManager::rejecting());
        assert!(c.sleep_mut().try_sleep(&TIMER).is_err());
        let (gate, _sleep) = c.into_parts();
        assert!(gate.is_powered());
    }
}
