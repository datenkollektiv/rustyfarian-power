//! The `try_sleep` sequence shared by both tiers.
//!
//! [`SleepAttemptMachine`] decides *when* each step of a rejecting deep-sleep
//! attempt happens; the tier decides *how*.
//! Feed it a [`SleepAttemptInput`], perform the returned [`SleepAttemptAction`], repeat,
//! until the machine answers [`SleepAttemptAction::Done`].
//! The machine owns no registers and does no IO: the tier arms the wake sources,
//! reads the hold register and the pins, enters sleep and releases the pins.
//! [`drive`] is that loop, generic over an IO-only [`SleepAttemptPort`], so the tiers
//! implement the port and keep no copy of the dispatch.
//!
//! # Sequence
//!
//! ```text
//! Start -> Arm(plan) -> Armed
//!   timer-only plan, or chip without a guard ........... -> Enter
//!   guard on every request (C2/C3, ESP32) .............. -> ReadPins
//!   guard only with the pad hold effective (S3) ........ -> ReadHold -> HoldEffective(b)
//!                                                           b: ReadPins, else Enter
//! PinLevels(l)  pins at their wake level -> Release -> Released -> Done(Err(Rejected))
//!               otherwise                -> Enter
//! Enter         Slept     -> Done(Ok(()))
//!               Returned  -> Release -> Released -> Done(Err(Rejected))
//! ```
//!
//! Every rejection after `Arm` passes through `Release`, also for a timer-only plan
//! (`pins: 0`), so the deadline cleanup is an explicit step.
//!
//! # Guard table
//!
//! [`ChipPolicy::guard`] is the one table both tiers share.
//! Pins are read right before entry because a hardware reject is useless where it
//! never returns or where the chip never rejects.
//!
//! | Chip | Pre-read of the wake pins |
//! |------|---------------------------|
//! | ESP32-C2, ESP32-C3 | always: the hardware sleeps and wakes at once on a held pin |
//! | ESP32 | always: EXT1 never rejects, and a rejected sleep hangs under the pad hold |
//! | ESP32-S3 | only when the global digital pad hold is effective ([`digital_hold_effective`](super::digital_hold_effective)) |
//! | ESP32-C6 | never (provisional): assumed to reject on its own; bench question: does a held EXT1 pin reject or wake at once |
//! | other chips | never: the hardware rejects on its own |
//!
//! A timer-only plan has no pins and never reads any.
//!
//! # Adapter contract
//!
//! Inputs that do not fit the current phase are ignored with [`SleepAttemptAction::Ignore`]
//! (a `PinLevels` nobody asked for, a second `Start`, anything after `Done`).
//! A machine runs one attempt; build a new one for a retry.
//!
//! - After [`SleepAttemptAction::Arm`] the tier configures the wake sources of
//!   [`SleepPlan`] and answers `Armed`; a configuration failure ends the attempt
//!   in the tier with its own error, the machine is dropped.
//! - Answer [`SleepAttemptAction::ReadHold`] with the raw `RTC_CNTL_DIG_ISO_REG`
//!   verdict, `HoldEffective(digital_hold_effective(reg))` (see [`digital_hold_effective`](super::digital_hold_effective)).
//! - Answer [`SleepAttemptAction::ReadPins`] with `PinLevels`, bit N set when GPIO N reads high.
//!   Read as late as possible, right before the answer.
//!   A pin whose level cannot be read (no input enable) is reported at the level
//!   opposite to its wake level, so it neither trips the `AnyHigh`/`AnyLow` guard nor
//!   the ESP32 `ALL_LOW` guard.
//! - Answer [`SleepAttemptAction::Enter`] with `Returned` when the hardware refused
//!   and the call came back, or `Slept` when it reported success.
//!   A sleep that is taken does not return at all.
//! - Answer [`SleepAttemptAction::Release`] with `Released` only after every cleanup step
//!   has run: undoing the arming (the tier's armed state: per-pin wake enables, holds, a
//!   software timer deadline) so that every wake pin of the plan is a plain digital input
//!   (ADR-003 point 1).
//!   `pins` is `0` for a timer-only plan; then only a software deadline, if the tier keeps one, is undone.
//!   Release is best-effort: the machine guarantees that cleanup runs before completion,
//!   not that every hardware restore succeeds; a failing step is logged by the tier and
//!   must not change the outcome; `Done(Err(Rejected))` follows regardless.
//!   `Release` always comes before that `Done`.
//! - On the esp-hal tier the whole loop runs inside one `critical_section::with`.
//!
//! [`drive`] adds two rules for the port: a failed [`arm`](SleepAttemptPort::arm) has
//! already attempted to disarm every wake source (a validation failure touches nothing; a
//! failure after a hardware write is rolled back best-effort through
//! [`arm_transactionally`](super::arm::arm_transactionally)), so no `Release` follows it, and a protocol error after a successful arm
//! releases the plan's pins before [`DriveError::Protocol`] is returned.

use super::{
    validate_gpio_level_source_against, validate_wake_sources, wake_pins_asserted, GpioWakeLevel,
    SleepError, SleepErrorKind, WakeSource, ESP32C3_WAKE_PIN_MASK, ESP32C6_WAKE_PIN_MASK,
    ESP32S3_WAKE_PIN_MASK, ESP32_WAKE_PIN_MASK,
};

/// Wake configuration of one attempt, extracted once from the request.
///
/// `pin_mask == 0` means the plan names no pins and `level` is a placeholder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SleepPlan {
    /// Timer wake delay in milliseconds, if the request has a timer source.
    pub timer: Option<u64>,
    /// Bitmask of the GPIO wake pins; zero when the request names none.
    pub pin_mask: u64,
    /// Wake level of `pin_mask`; [`GpioWakeLevel::AnyHigh`] when the mask is zero.
    pub level: GpioWakeLevel,
}

impl SleepPlan {
    /// Returns `true` when the plan names at least one wake pin.
    #[must_use]
    pub const fn has_pins(&self) -> bool {
        self.pin_mask != 0
    }
}

/// When the pins are read before sleep entry; see the guard table in the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreReadGuard {
    /// Never read the pins.
    Never,
    /// Read the pins on every request that names pins.
    Always,
    /// Read the pins only while the global digital pad hold is effective.
    WhenHoldEffective,
}

/// The chip the attempt runs on, passed by the tier at run time.
///
/// Not cfg-gated: every row compiles and is tested on the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ChipPolicy {
    /// Original ESP32 (EXT1 has no any-low mode; multi-pin `AnyLow` is unsupported).
    Esp32,
    /// ESP32-S3.
    Esp32S3,
    /// ESP32-C2.
    Esp32C2,
    /// ESP32-C3.
    Esp32C3,
    /// ESP32-C6.
    Esp32C6,
    /// Any other chip: no software guard.
    Other,
}

impl ChipPolicy {
    /// The pre-read guard of this chip.
    #[must_use]
    pub const fn guard(self) -> PreReadGuard {
        match self {
            Self::Esp32 | Self::Esp32C2 | Self::Esp32C3 => PreReadGuard::Always,
            Self::Esp32S3 => PreReadGuard::WhenHoldEffective,
            Self::Esp32C6 | Self::Other => PreReadGuard::Never,
        }
    }

    /// The valid deep-sleep wake-pin mask of this chip; `None` when the chip is not tabled.
    ///
    /// [`SleepAttemptMachine::new`] range-checks a `GpioLevel` mask against it.
    #[must_use]
    pub const fn wake_pin_mask(self) -> Option<u64> {
        match self {
            Self::Esp32 => Some(ESP32_WAKE_PIN_MASK),
            Self::Esp32S3 => Some(ESP32S3_WAKE_PIN_MASK),
            Self::Esp32C2 | Self::Esp32C3 => Some(ESP32C3_WAKE_PIN_MASK),
            Self::Esp32C6 => Some(ESP32C6_WAKE_PIN_MASK),
            Self::Other => None,
        }
    }

    /// Returns `true` when `AnyLow` over several pins has any-low semantics on this chip.
    ///
    /// `false` on the original ESP32, where EXT1 combines low pads with AND;
    /// [`SleepAttemptMachine::new`] rejects that request on both tiers.
    #[must_use]
    pub const fn any_low_multi_pin(self) -> bool {
        !matches!(self, Self::Esp32)
    }

    /// Returns `true` when `levels` shows the hardware reject condition on this chip.
    fn asserted(plan: &SleepPlan, levels: u64) -> bool {
        wake_pins_asserted(levels, plan.pin_mask, plan.level)
    }
}

/// Event fed to [`SleepAttemptMachine::poll`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SleepAttemptInput {
    /// Begin the attempt; answered with [`SleepAttemptAction::Arm`].
    Start,
    /// The wake sources of the plan are configured.
    Armed,
    /// Verdict of [`digital_hold_effective`](super::digital_hold_effective) over the hold register.
    HoldEffective(bool),
    /// Pin levels, bit N set when GPIO N reads high.
    PinLevels(u64),
    /// The sleep call came back: the hardware rejected the sleep.
    Returned,
    /// The sleep call reported success.
    Slept,
    /// The wake pins are restored as plain inputs.
    Released,
}

/// Step the tier performs after [`SleepAttemptMachine::poll`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SleepAttemptAction {
    /// Configure the plan's wake sources, then feed [`SleepAttemptInput::Armed`].
    Arm(SleepPlan),
    /// Read the hold register, then feed [`SleepAttemptInput::HoldEffective`].
    ReadHold,
    /// Read the pins of `mask`, then feed [`SleepAttemptInput::PinLevels`].
    ReadPins {
        /// Pins to read.
        mask: u64,
    },
    /// Enter deep sleep, then feed `Returned` or `Slept`.
    Enter,
    /// Undo the arming and restore `pins` as plain inputs, then feed
    /// [`SleepAttemptInput::Released`]; `pins` is `0` for a timer-only plan.
    Release {
        /// Pins to restore.
        pins: u64,
    },
    /// The attempt is over; the error kind is always [`SleepErrorKind::Rejected`].
    Done(Result<(), SleepErrorKind>),
    /// The input did not fit the current phase; do nothing.
    Ignore,
}

/// How the sleep call ended, as reported by [`SleepAttemptPort::enter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entered {
    /// The call reported success.
    Slept,
    /// The call came back: the hardware rejected the sleep.
    Returned,
}

/// IO of one attempt, one method per [`SleepAttemptAction`]; no sequencing.
///
/// Implemented privately by each tier and consumed by [`drive`].
/// The order of the calls is the machine's, not the port's.
#[doc(hidden)]
pub trait SleepAttemptPort {
    /// The tier's error for a failed [`arm`](Self::arm).
    type Error;

    /// Configures the wake sources of `plan`.
    ///
    /// A failed arm has already attempted to disarm every wake source, so [`drive`] issues no
    /// `release` after it: a validation failure touches no hardware, and a failure after the
    /// first hardware write is rolled back best-effort, see
    /// [`arm_transactionally`](super::arm::arm_transactionally).
    ///
    /// # Errors
    ///
    /// The tier's error when a wake source cannot be configured.
    fn arm(&mut self, plan: SleepPlan) -> Result<(), Self::Error>;

    /// Reads the hold register; `true` when
    /// [`digital_hold_effective`](super::digital_hold_effective) says the pad hold is effective.
    fn read_hold(&mut self) -> bool;

    /// Reads the pins of `mask`, bit N set when GPIO N reads high; as late as possible.
    fn read_pins(&mut self, mask: u64) -> u64;

    /// Enters deep sleep; a taken sleep does not return.
    fn enter(&mut self) -> Entered;

    /// Undoes the arming and restores `pins` as plain digital inputs, best-effort.
    ///
    /// Returns only after every cleanup step has run; a step that fails is logged by the
    /// tier and does not change the outcome.
    /// [`drive`] guarantees that cleanup runs before the attempt completes, not that every
    /// hardware restore succeeds; the ADR-003 point 1 postcondition holds when it does.
    /// `pins` is `0` for a timer-only plan.
    fn release(&mut self, pins: u64);
}

/// Why [`drive`] ended without a successful sleep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveError<E> {
    /// [`SleepAttemptPort::arm`] failed; nothing was released.
    Arm(E),
    /// The attempt ended rejected with this kind, after the release step.
    Rejected(SleepErrorKind),
    /// The machine ignored an input: an adapter bug. The plan's pins were released if armed.
    Protocol,
}

/// Runs one attempt: performs the machine's actions on `port` until it is done.
///
/// # Errors
///
/// - [`DriveError::Arm`] when the port fails to arm.
/// - [`DriveError::Rejected`] when the attempt was rejected, after `release`.
/// - [`DriveError::Protocol`] when the machine ignored an input.
pub fn drive<P: SleepAttemptPort>(
    mut machine: SleepAttemptMachine,
    port: &mut P,
) -> Result<(), DriveError<P::Error>> {
    let first = machine.poll(SleepAttemptInput::Start);
    run(machine, port, first)
}

/// The dispatch loop of [`drive`], starting from `action`.
fn run<P: SleepAttemptPort>(
    mut machine: SleepAttemptMachine,
    port: &mut P,
    mut action: SleepAttemptAction,
) -> Result<(), DriveError<P::Error>> {
    let mut armed = false;
    let mut released = false;
    loop {
        let input = match action {
            SleepAttemptAction::Arm(plan) => {
                port.arm(plan).map_err(DriveError::Arm)?;
                armed = true;
                SleepAttemptInput::Armed
            }
            SleepAttemptAction::ReadHold => SleepAttemptInput::HoldEffective(port.read_hold()),
            SleepAttemptAction::ReadPins { mask } => {
                SleepAttemptInput::PinLevels(port.read_pins(mask))
            }
            SleepAttemptAction::Enter => match port.enter() {
                Entered::Slept => SleepAttemptInput::Slept,
                Entered::Returned => SleepAttemptInput::Returned,
            },
            SleepAttemptAction::Release { pins } => {
                port.release(pins);
                released = true;
                SleepAttemptInput::Released
            }
            SleepAttemptAction::Done(Ok(())) => return Ok(()),
            SleepAttemptAction::Done(Err(kind)) => return Err(DriveError::Rejected(kind)),
            SleepAttemptAction::Ignore => {
                if armed && !released {
                    port.release(machine.plan().pin_mask);
                }
                return Err(DriveError::Protocol);
            }
        };
        action = machine.poll(input);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Arming,
    ReadingHold,
    ReadingPins,
    Entering,
    Releasing,
    Finished,
}

/// Sans-IO state machine of one `try_sleep` attempt.
///
/// See the module docs for the sequence and the adapter contract.
#[derive(Debug, Clone, Copy)]
pub struct SleepAttemptMachine {
    chip: ChipPolicy,
    plan: SleepPlan,
    phase: Phase,
}

impl SleepAttemptMachine {
    /// Validates `sources` and extracts the plan.
    ///
    /// # Errors
    ///
    /// - Every error of [`validate_wake_sources`].
    /// - [`SleepError::GpioMaskZero`] when a `GpioLevel` source names no pins.
    /// - [`SleepError::GpioMaskOutOfRange`] when the pins are outside
    ///   [`ChipPolicy::wake_pin_mask`]; a chip without a mask is not range-checked.
    /// - [`SleepError::AnyLowMultiplePins`] for `AnyLow` over several pins where
    ///   [`ChipPolicy::any_low_multi_pin`] is `false` (the original ESP32).
    pub fn new(chip: ChipPolicy, sources: &[WakeSource]) -> Result<Self, SleepError> {
        validate_wake_sources(sources)?;
        let mut plan = SleepPlan {
            timer: None,
            pin_mask: 0,
            level: GpioWakeLevel::AnyHigh,
        };
        for source in sources {
            match *source {
                WakeSource::Timer { duration_ms } => plan.timer = Some(duration_ms),
                WakeSource::GpioLevel { pin_mask, level } => {
                    if pin_mask == 0 {
                        return Err(SleepError::GpioMaskZero);
                    }
                    if let Some(valid_mask) = chip.wake_pin_mask() {
                        validate_gpio_level_source_against(pin_mask, valid_mask)?;
                    }
                    if !chip.any_low_multi_pin()
                        && level == GpioWakeLevel::AnyLow
                        && pin_mask.count_ones() > 1
                    {
                        return Err(SleepError::AnyLowMultiplePins);
                    }
                    plan.pin_mask = pin_mask;
                    plan.level = level;
                }
            }
        }
        Ok(Self {
            chip,
            plan,
            phase: Phase::Idle,
        })
    }

    /// The plan extracted from the request.
    #[must_use]
    pub const fn plan(&self) -> SleepPlan {
        self.plan
    }

    /// Returns `true` once the machine answered [`SleepAttemptAction::Done`].
    #[must_use]
    pub const fn is_done(&self) -> bool {
        matches!(self.phase, Phase::Finished)
    }

    /// Feeds one input and returns the step the tier performs next.
    pub fn poll(&mut self, input: SleepAttemptInput) -> SleepAttemptAction {
        match (self.phase, input) {
            (Phase::Idle, SleepAttemptInput::Start) => {
                self.phase = Phase::Arming;
                SleepAttemptAction::Arm(self.plan)
            }
            (Phase::Arming, SleepAttemptInput::Armed) => {
                if !self.plan.has_pins() {
                    return self.enter();
                }
                match self.chip.guard() {
                    PreReadGuard::Never => self.enter(),
                    PreReadGuard::Always => self.read_pins(),
                    PreReadGuard::WhenHoldEffective => {
                        self.phase = Phase::ReadingHold;
                        SleepAttemptAction::ReadHold
                    }
                }
            }
            (Phase::ReadingHold, SleepAttemptInput::HoldEffective(effective)) => {
                if effective {
                    self.read_pins()
                } else {
                    self.enter()
                }
            }
            (Phase::ReadingPins, SleepAttemptInput::PinLevels(levels)) => {
                if ChipPolicy::asserted(&self.plan, levels) {
                    self.release()
                } else {
                    self.enter()
                }
            }
            (Phase::Entering, SleepAttemptInput::Slept) => self.finish(Ok(())),
            (Phase::Entering, SleepAttemptInput::Returned) => self.release(),
            (Phase::Releasing, SleepAttemptInput::Released) => {
                self.finish(Err(SleepErrorKind::Rejected))
            }
            _ => SleepAttemptAction::Ignore,
        }
    }

    fn enter(&mut self) -> SleepAttemptAction {
        self.phase = Phase::Entering;
        SleepAttemptAction::Enter
    }

    fn read_pins(&mut self) -> SleepAttemptAction {
        self.phase = Phase::ReadingPins;
        SleepAttemptAction::ReadPins {
            mask: self.plan.pin_mask,
        }
    }

    /// Undo the arming; `pins` is zero for a timer-only plan.
    fn release(&mut self) -> SleepAttemptAction {
        self.phase = Phase::Releasing;
        SleepAttemptAction::Release {
            pins: self.plan.pin_mask,
        }
    }

    fn finish(&mut self, result: Result<(), SleepErrorKind>) -> SleepAttemptAction {
        self.phase = Phase::Finished;
        SleepAttemptAction::Done(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sleep::Error;
    use std::vec::Vec;
    use SleepAttemptAction as A;
    use SleepAttemptInput as I;

    const CHIPS: [ChipPolicy; 6] = [
        ChipPolicy::Esp32,
        ChipPolicy::Esp32S3,
        ChipPolicy::Esp32C2,
        ChipPolicy::Esp32C3,
        ChipPolicy::Esp32C6,
        ChipPolicy::Other,
    ];
    const PIN2: u64 = 1 << 2;
    const PINS: u64 = (1 << 2) | (1 << 4);

    fn pins(level: GpioWakeLevel) -> [WakeSource; 2] {
        [
            WakeSource::Timer { duration_ms: 5_000 },
            WakeSource::GpioLevel {
                pin_mask: PINS,
                level,
            },
        ]
    }

    fn started(chip: ChipPolicy, sources: &[WakeSource]) -> SleepAttemptMachine {
        let mut m = SleepAttemptMachine::new(chip, sources).unwrap();
        assert!(matches!(m.poll(I::Start), A::Arm(_)));
        m
    }

    #[test]
    fn guard_table() {
        assert_eq!(ChipPolicy::Esp32.guard(), PreReadGuard::Always);
        assert_eq!(ChipPolicy::Esp32C2.guard(), PreReadGuard::Always);
        assert_eq!(ChipPolicy::Esp32C3.guard(), PreReadGuard::Always);
        assert_eq!(ChipPolicy::Esp32S3.guard(), PreReadGuard::WhenHoldEffective);
        assert_eq!(ChipPolicy::Esp32C6.guard(), PreReadGuard::Never);
        assert_eq!(ChipPolicy::Other.guard(), PreReadGuard::Never);
    }

    #[test]
    fn wake_pin_mask_table() {
        assert_eq!(ChipPolicy::Esp32.wake_pin_mask(), Some(ESP32_WAKE_PIN_MASK));
        assert_eq!(
            ChipPolicy::Esp32S3.wake_pin_mask(),
            Some(ESP32S3_WAKE_PIN_MASK)
        );
        assert_eq!(
            ChipPolicy::Esp32C2.wake_pin_mask(),
            Some(ESP32C3_WAKE_PIN_MASK)
        );
        assert_eq!(
            ChipPolicy::Esp32C3.wake_pin_mask(),
            Some(ESP32C3_WAKE_PIN_MASK)
        );
        assert_eq!(
            ChipPolicy::Esp32C6.wake_pin_mask(),
            Some(ESP32C6_WAKE_PIN_MASK)
        );
        assert_eq!(ChipPolicy::Other.wake_pin_mask(), None);
    }

    #[test]
    fn out_of_range_mask_is_rejected_per_chip() {
        for chip in CHIPS {
            let Some(valid_mask) = chip.wake_pin_mask() else {
                // No table row: any mask passes the range check.
                assert!(SleepAttemptMachine::new(chip, &pins(GpioWakeLevel::AnyHigh)).is_ok());
                continue;
            };
            let pin_mask = valid_mask | (1 << 63);
            let err = SleepAttemptMachine::new(
                chip,
                &[WakeSource::GpioLevel {
                    pin_mask,
                    level: GpioWakeLevel::AnyHigh,
                }],
            )
            .unwrap_err();
            assert_eq!(
                err,
                SleepError::GpioMaskOutOfRange {
                    pin_mask,
                    valid_mask
                },
                "{chip:?}"
            );
            let lowest = 1u64 << valid_mask.trailing_zeros();
            assert!(SleepAttemptMachine::new(
                chip,
                &[WakeSource::GpioLevel {
                    pin_mask: lowest,
                    level: GpioWakeLevel::AnyLow
                }]
            )
            .is_ok());
        }
    }

    #[test]
    fn c3_and_s3_reject_each_others_pins() {
        let gpio = |pin_mask| {
            [WakeSource::GpioLevel {
                pin_mask,
                level: GpioWakeLevel::AnyHigh,
            }]
        };
        assert!(SleepAttemptMachine::new(ChipPolicy::Esp32S3, &gpio(1 << 10)).is_ok());
        assert!(matches!(
            SleepAttemptMachine::new(ChipPolicy::Esp32C3, &gpio(1 << 10)),
            Err(SleepError::GpioMaskOutOfRange {
                valid_mask: 0x3F,
                ..
            })
        ));
        // Pin 21 is an S3 wake pin but not an original-ESP32 one.
        assert!(SleepAttemptMachine::new(ChipPolicy::Esp32S3, &gpio(1 << 21)).is_ok());
        assert!(matches!(
            SleepAttemptMachine::new(ChipPolicy::Esp32, &gpio(1 << 21)),
            Err(SleepError::GpioMaskOutOfRange { .. })
        ));
    }

    #[test]
    fn error_order_is_stable_on_a_ranged_chip() {
        let new = |s: &[WakeSource]| SleepAttemptMachine::new(ChipPolicy::Esp32C3, s).unwrap_err();
        let out_of_range = WakeSource::GpioLevel {
            pin_mask: 1 << 40,
            level: GpioWakeLevel::AnyHigh,
        };
        let zero = WakeSource::GpioLevel {
            pin_mask: 0,
            level: GpioWakeLevel::AnyHigh,
        };
        let t = WakeSource::Timer { duration_ms: 1 };
        assert_eq!(new(&[]), SleepError::NoWakeSource);
        assert_eq!(
            new(&[t, t, out_of_range]),
            SleepError::MultipleTimers { count: 2 }
        );
        assert!(matches!(
            new(&[
                WakeSource::Timer {
                    duration_ms: u64::MAX
                },
                out_of_range
            ]),
            SleepError::TimerTooLong { .. }
        ));
        assert_eq!(new(&[zero]), SleepError::GpioMaskZero);
        assert!(matches!(
            new(&[out_of_range]),
            SleepError::GpioMaskOutOfRange { .. }
        ));
    }

    #[test]
    fn plan_is_extracted_once() {
        let m =
            SleepAttemptMachine::new(ChipPolicy::Esp32C3, &pins(GpioWakeLevel::AnyLow)).unwrap();
        assert_eq!(
            m.plan(),
            SleepPlan {
                timer: Some(5_000),
                pin_mask: PINS,
                level: GpioWakeLevel::AnyLow,
            }
        );
        let t =
            SleepAttemptMachine::new(ChipPolicy::Esp32, &[WakeSource::Timer { duration_ms: 1 }])
                .unwrap();
        assert!(!t.plan().has_pins());
        assert_eq!(t.plan().timer, Some(1));
    }

    #[test]
    fn validation_errors() {
        let err = |s: &[WakeSource]| SleepAttemptMachine::new(ChipPolicy::Esp32S3, s).unwrap_err();
        assert_eq!(err(&[]), SleepError::NoWakeSource);
        let t = WakeSource::Timer { duration_ms: 1 };
        assert_eq!(err(&[t, t]), SleepError::MultipleTimers { count: 2 });
        let g = WakeSource::GpioLevel {
            pin_mask: 1,
            level: GpioWakeLevel::AnyHigh,
        };
        assert_eq!(err(&[g, g]), SleepError::MultipleGpioLevel);
        assert!(matches!(
            err(&[WakeSource::Timer {
                duration_ms: u64::MAX
            }]),
            SleepError::TimerTooLong { .. }
        ));
        assert_eq!(
            err(&[WakeSource::GpioLevel {
                pin_mask: 0,
                level: GpioWakeLevel::AnyHigh
            }]),
            SleepError::GpioMaskZero
        );
    }

    #[test]
    fn timer_only_skips_pin_reads_on_every_chip() {
        for chip in CHIPS {
            let mut m = started(chip, &[WakeSource::Timer { duration_ms: 10 }]);
            assert_eq!(m.poll(I::Armed), A::Enter, "{chip:?}");
            assert_eq!(m.poll(I::Returned), A::Release { pins: 0 }, "{chip:?}");
            assert!(!m.is_done());
            assert_eq!(m.poll(I::Released), A::Done(Err(SleepErrorKind::Rejected)));
            assert!(m.is_done());
        }
    }

    #[test]
    fn timer_only_slept_is_ok() {
        let mut m = started(
            ChipPolicy::Esp32C3,
            &[WakeSource::Timer { duration_ms: 10 }],
        );
        assert_eq!(m.poll(I::Armed), A::Enter);
        assert_eq!(m.poll(I::Slept), A::Done(Ok(())));
    }

    /// The pins a chip accepts for `level`: the original ESP32 takes one pin for `AnyLow`.
    fn mask_for(chip: ChipPolicy, level: GpioWakeLevel) -> u64 {
        if chip == ChipPolicy::Esp32 && level == GpioWakeLevel::AnyLow {
            PIN2
        } else {
            PINS
        }
    }

    /// Drives the pin-guarded path up to the pin read of a chip that reads at all.
    fn to_pin_read(chip: ChipPolicy, level: GpioWakeLevel) -> SleepAttemptMachine {
        let mask = mask_for(chip, level);
        let mut m = started(
            chip,
            &[WakeSource::GpioLevel {
                pin_mask: mask,
                level,
            }],
        );
        match m.poll(I::Armed) {
            A::ReadPins { mask: read } => assert_eq!(read, mask),
            A::ReadHold => {
                assert_eq!(chip, ChipPolicy::Esp32S3);
                assert_eq!(m.poll(I::HoldEffective(true)), A::ReadPins { mask });
            }
            other => panic!("{chip:?}: unexpected {other:?}"),
        }
        m
    }

    #[test]
    fn guarded_chips_trip_then_release_then_reject() {
        for chip in CHIPS {
            if chip.guard() == PreReadGuard::Never {
                continue;
            }
            // AnyHigh: one pin high trips. AnyLow: one pin low trips.
            let mut m = to_pin_read(chip, GpioWakeLevel::AnyHigh);
            assert_eq!(m.poll(I::PinLevels(PIN2)), A::Release { pins: PINS });
            assert!(!m.is_done());
            assert_eq!(
                m.poll(I::Released),
                A::Done(Err(SleepErrorKind::Rejected)),
                "{chip:?}"
            );

            let mut m = to_pin_read(chip, GpioWakeLevel::AnyLow);
            let mask = mask_for(chip, GpioWakeLevel::AnyLow);
            assert_eq!(
                m.poll(I::PinLevels(PINS & !PIN2)),
                A::Release { pins: mask }
            );
            assert_eq!(m.poll(I::Released), A::Done(Err(SleepErrorKind::Rejected)));
        }
    }

    #[test]
    fn guarded_chips_pass_when_no_pin_asserted() {
        for chip in CHIPS {
            if chip.guard() == PreReadGuard::Never {
                continue;
            }
            let mut m = to_pin_read(chip, GpioWakeLevel::AnyHigh);
            assert_eq!(m.poll(I::PinLevels(0)), A::Enter, "{chip:?}");
            let mut m = to_pin_read(chip, GpioWakeLevel::AnyLow);
            assert_eq!(m.poll(I::PinLevels(PINS)), A::Enter, "{chip:?}");
        }
    }

    #[test]
    fn any_low_multi_pin_table() {
        for chip in CHIPS {
            assert_eq!(
                chip.any_low_multi_pin(),
                chip != ChipPolicy::Esp32,
                "{chip:?}"
            );
        }
    }

    #[test]
    fn multi_pin_any_low_is_refused_only_where_unsupported() {
        for chip in CHIPS {
            let multi = SleepAttemptMachine::new(chip, &pins(GpioWakeLevel::AnyLow));
            if chip.any_low_multi_pin() {
                assert!(multi.is_ok(), "{chip:?}");
            } else {
                let err = multi.unwrap_err();
                assert_eq!(err, SleepError::AnyLowMultiplePins);
                assert_eq!(err.kind(), SleepErrorKind::Unsupported);
            }
            assert!(
                SleepAttemptMachine::new(chip, &pins(GpioWakeLevel::AnyHigh)).is_ok(),
                "{chip:?}"
            );
            assert!(
                SleepAttemptMachine::new(
                    chip,
                    &[WakeSource::GpioLevel {
                        pin_mask: PIN2,
                        level: GpioWakeLevel::AnyLow
                    }]
                )
                .is_ok(),
                "{chip:?}"
            );
        }
    }

    #[test]
    fn esp32_single_pin_any_low_trips_on_a_low_pin() {
        let mut m = to_pin_read(ChipPolicy::Esp32, GpioWakeLevel::AnyLow);
        assert_eq!(m.poll(I::PinLevels(0)), A::Release { pins: PIN2 });
        let mut m = to_pin_read(ChipPolicy::Esp32, GpioWakeLevel::AnyLow);
        assert_eq!(m.poll(I::PinLevels(PIN2)), A::Enter);
    }

    #[test]
    fn esp32_reads_pins_even_with_no_hold_read() {
        let mut m = started(ChipPolicy::Esp32, &pins(GpioWakeLevel::AnyHigh));
        assert_eq!(m.poll(I::Armed), A::ReadPins { mask: PINS });
    }

    #[test]
    fn s3_skips_pin_read_without_effective_hold() {
        let mut m = started(ChipPolicy::Esp32S3, &pins(GpioWakeLevel::AnyHigh));
        assert_eq!(m.poll(I::Armed), A::ReadHold);
        assert_eq!(m.poll(I::HoldEffective(false)), A::Enter);
    }

    #[test]
    fn unguarded_chips_go_straight_to_enter() {
        for chip in [ChipPolicy::Esp32C6, ChipPolicy::Other] {
            let mut m = started(chip, &pins(GpioWakeLevel::AnyHigh));
            assert_eq!(m.poll(I::Armed), A::Enter);
        }
    }

    #[test]
    fn hardware_return_releases_pins_before_done() {
        for chip in CHIPS {
            let mut m = started(chip, &pins(GpioWakeLevel::AnyHigh));
            let mut action = m.poll(I::Armed);
            loop {
                action = match action {
                    A::ReadHold => m.poll(I::HoldEffective(true)),
                    A::ReadPins { .. } => m.poll(I::PinLevels(0)),
                    A::Enter => break,
                    other => panic!("{chip:?}: unexpected {other:?}"),
                };
            }
            assert_eq!(m.poll(I::Returned), A::Release { pins: PINS }, "{chip:?}");
            assert!(!m.is_done());
            assert_eq!(m.poll(I::Released), A::Done(Err(SleepErrorKind::Rejected)));
            assert!(m.is_done());
        }
    }

    #[test]
    fn pins_slept_is_ok() {
        let mut m = started(ChipPolicy::Esp32C6, &pins(GpioWakeLevel::AnyHigh));
        assert_eq!(m.poll(I::Armed), A::Enter);
        assert_eq!(m.poll(I::Slept), A::Done(Ok(())));
    }

    #[test]
    fn out_of_phase_inputs_are_ignored() {
        let mut m =
            SleepAttemptMachine::new(ChipPolicy::Esp32S3, &pins(GpioWakeLevel::AnyHigh)).unwrap();
        for input in [
            I::Armed,
            I::HoldEffective(true),
            I::PinLevels(0),
            I::Returned,
            I::Slept,
            I::Released,
        ] {
            assert_eq!(m.poll(input), A::Ignore, "{input:?} before Start");
        }
        assert!(matches!(m.poll(I::Start), A::Arm(_)));
        assert_eq!(m.poll(I::Start), A::Ignore);
        assert_eq!(m.poll(I::PinLevels(PINS)), A::Ignore);
        assert_eq!(m.poll(I::Armed), A::ReadHold);
        assert_eq!(m.poll(I::Armed), A::Ignore);
        assert_eq!(m.poll(I::PinLevels(PINS)), A::Ignore);
        assert_eq!(m.poll(I::HoldEffective(true)), A::ReadPins { mask: PINS });
        assert_eq!(m.poll(I::HoldEffective(true)), A::Ignore);
        assert_eq!(m.poll(I::Returned), A::Ignore);
        assert_eq!(m.poll(I::PinLevels(0)), A::Enter);
        assert_eq!(m.poll(I::PinLevels(0)), A::Ignore);
        assert_eq!(m.poll(I::Released), A::Ignore);
        assert_eq!(m.poll(I::Returned), A::Release { pins: PINS });
        assert_eq!(m.poll(I::Returned), A::Ignore);
        assert_eq!(m.poll(I::Released), A::Done(Err(SleepErrorKind::Rejected)));
        for input in [I::Start, I::Armed, I::Returned, I::Slept, I::Released] {
            assert_eq!(m.poll(input), A::Ignore, "{input:?} after Done");
        }
    }

    #[test]
    fn unrelated_pin_levels_are_ignored_by_the_guard() {
        let mut m = to_pin_read(ChipPolicy::Esp32C3, GpioWakeLevel::AnyHigh);
        assert_eq!(m.poll(I::PinLevels(!PINS)), A::Enter);
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Call {
        Arm(SleepPlan),
        ReadHold,
        ReadPins(u64),
        Enter,
        Release(u64),
    }

    /// Records every call; answers from the fields.
    struct FakePort {
        calls: Vec<Call>,
        hold: bool,
        levels: u64,
        entered: Entered,
        arm_error: Option<&'static str>,
    }

    impl FakePort {
        fn new(entered: Entered) -> Self {
            Self {
                calls: Vec::new(),
                hold: true,
                levels: 0,
                entered,
                arm_error: None,
            }
        }
    }

    impl SleepAttemptPort for FakePort {
        type Error = &'static str;

        fn arm(&mut self, plan: SleepPlan) -> Result<(), Self::Error> {
            self.calls.push(Call::Arm(plan));
            self.arm_error.map_or(Ok(()), Err)
        }

        fn read_hold(&mut self) -> bool {
            self.calls.push(Call::ReadHold);
            self.hold
        }

        fn read_pins(&mut self, mask: u64) -> u64 {
            self.calls.push(Call::ReadPins(mask));
            self.levels
        }

        fn enter(&mut self) -> Entered {
            self.calls.push(Call::Enter);
            self.entered
        }

        fn release(&mut self, pins: u64) {
            self.calls.push(Call::Release(pins));
        }
    }

    fn machine(chip: ChipPolicy, sources: &[WakeSource]) -> SleepAttemptMachine {
        SleepAttemptMachine::new(chip, sources).unwrap()
    }

    #[test]
    fn drive_software_reject_releases_without_entering() {
        let m = machine(ChipPolicy::Esp32C3, &pins(GpioWakeLevel::AnyHigh));
        let plan = m.plan();
        let mut port = FakePort::new(Entered::Slept);
        port.levels = PIN2;
        assert_eq!(
            drive(m, &mut port),
            Err(DriveError::Rejected(SleepErrorKind::Rejected))
        );
        assert_eq!(
            port.calls,
            [Call::Arm(plan), Call::ReadPins(PINS), Call::Release(PINS)]
        );
    }

    #[test]
    fn drive_hardware_reject_releases_after_enter() {
        let m = machine(ChipPolicy::Esp32S3, &pins(GpioWakeLevel::AnyHigh));
        let plan = m.plan();
        let mut port = FakePort::new(Entered::Returned);
        assert_eq!(
            drive(m, &mut port),
            Err(DriveError::Rejected(SleepErrorKind::Rejected))
        );
        assert_eq!(
            port.calls,
            [
                Call::Arm(plan),
                Call::ReadHold,
                Call::ReadPins(PINS),
                Call::Enter,
                Call::Release(PINS)
            ]
        );
    }

    #[test]
    fn drive_timer_only_reject_passes_through_release() {
        let m = machine(
            ChipPolicy::Esp32C3,
            &[WakeSource::Timer { duration_ms: 10 }],
        );
        let plan = m.plan();
        let mut port = FakePort::new(Entered::Returned);
        assert_eq!(
            drive(m, &mut port),
            Err(DriveError::Rejected(SleepErrorKind::Rejected))
        );
        assert_eq!(port.calls, [Call::Arm(plan), Call::Enter, Call::Release(0)]);
    }

    #[test]
    fn drive_arm_failure_returns_the_error_without_release() {
        let m = machine(ChipPolicy::Esp32C3, &pins(GpioWakeLevel::AnyHigh));
        let plan = m.plan();
        let mut port = FakePort::new(Entered::Slept);
        port.arm_error = Some("arm failed");
        assert_eq!(drive(m, &mut port), Err(DriveError::Arm("arm failed")));
        assert_eq!(port.calls, [Call::Arm(plan)]);
    }

    #[test]
    fn drive_slept_ends_ok_without_release() {
        let m = machine(ChipPolicy::Esp32C6, &pins(GpioWakeLevel::AnyHigh));
        let plan = m.plan();
        let mut port = FakePort::new(Entered::Slept);
        assert_eq!(drive(m, &mut port), Ok(()));
        assert_eq!(port.calls, [Call::Arm(plan), Call::Enter]);
    }

    #[test]
    fn drive_protocol_error_before_arm_releases_nothing() {
        let mut machine = machine(ChipPolicy::Esp32C6, &pins(GpioWakeLevel::AnyHigh));
        machine.poll(I::Start);
        // A mid-attempt machine ignores `Start`.
        let mut port = FakePort::new(Entered::Slept);
        assert_eq!(drive(machine, &mut port), Err(DriveError::Protocol));
        assert!(port.calls.is_empty());
    }

    #[test]
    fn drive_protocol_error_after_arm_releases_the_plan_pins() {
        // The machine already left `Arming`, so it ignores the `Armed` that follows the arm.
        let mut machine = machine(ChipPolicy::Esp32C6, &pins(GpioWakeLevel::AnyHigh));
        let plan = machine.plan();
        machine.poll(I::Start);
        machine.poll(I::Armed);
        let mut port = FakePort::new(Entered::Slept);
        assert_eq!(
            run(machine, &mut port, A::Arm(plan)),
            Err(DriveError::Protocol)
        );
        assert_eq!(port.calls, [Call::Arm(plan), Call::Release(PINS)]);
    }
}
