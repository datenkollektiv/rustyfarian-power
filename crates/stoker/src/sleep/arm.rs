//! Transactional arming of one sleep attempt's wake sources.
//!
//! [`arm_transactionally`] decides *in which order* the wake sources are applied and *what
//! happens when a step fails*; the tier decides *how* each step talks to the hardware.
//! Implement [`WakeArmOps`] with one SDK or register operation per method and call
//! [`arm_transactionally`] from [`SleepAttemptPort::arm`](super::attempt::SleepAttemptPort::arm).
//!
//! # Order and failure policy
//!
//! - Apply: [`clear`](WakeArmOps::clear), then [`arm_timer`](WakeArmOps::arm_timer) when the plan
//!   has a timer, then [`arm_pins`](WakeArmOps::arm_pins) when it names pins, then
//!   [`finish`](WakeArmOps::finish).
//!   The first failing step stops the apply phase.
//! - Roll back after any apply failure: [`undo_sources`](WakeArmOps::undo_sources), then
//!   [`undo_pin`](WakeArmOps::undo_pin) for every pin [`armed_pins`](WakeArmOps::armed_pins)
//!   reports.
//!   Every rollback step is attempted, whatever an earlier one returned; the tier logs a
//!   failing step.
//! - The apply error is returned unchanged, even when a rollback step fails.
//!
//! Rollback is best-effort: it attempts to disarm every wake source; it cannot guarantee
//! that every hardware call succeeds.
//! Validation that needs no hardware belongs before [`arm_transactionally`], so a
//! validation failure touches nothing.

use super::attempt::SleepPlan;
use super::GpioWakeLevel;

/// Hardware operations of one transactional arm, implemented privately by a tier.
///
/// Each method is one hardware operation and makes no decision; the order and the failure
/// policy are [`arm_transactionally`]'s.
#[doc(hidden)]
pub trait WakeArmOps {
    /// The tier's error for a failed operation.
    type Error;

    /// Drops wake configuration left from an earlier attempt, keeping the per-pin arming of
    /// `keep_pins`.
    ///
    /// # Errors
    ///
    /// The tier's error when the hardware refuses.
    fn clear(&mut self, keep_pins: u64) -> Result<(), Self::Error>;

    /// Arms the timer wake source.
    ///
    /// # Errors
    ///
    /// The tier's error when the hardware refuses.
    fn arm_timer(&mut self, duration_ms: u64) -> Result<(), Self::Error>;

    /// Arms the GPIO wake source for `pin_mask` at `level`.
    ///
    /// # Errors
    ///
    /// The tier's error when the hardware refuses.
    fn arm_pins(&mut self, pin_mask: u64, level: GpioWakeLevel) -> Result<(), Self::Error>;

    /// Final configuration after the wake sources, such as GPIO isolation.
    ///
    /// # Errors
    ///
    /// The tier's error when the hardware refuses.
    fn finish(&mut self) -> Result<(), Self::Error>;

    /// Rollback: disables every chip-wide wake source.
    ///
    /// # Errors
    ///
    /// The tier's error; [`arm_transactionally`] continues with the pins regardless.
    fn undo_sources(&mut self) -> Result<(), Self::Error>;

    /// Rollback: the pins whose per-pin wake enable is still set, bit N = GPIO N.
    fn armed_pins(&mut self) -> u64;

    /// Rollback: clears the per-pin wake enable of `pin`.
    ///
    /// # Errors
    ///
    /// The tier's error; [`arm_transactionally`] continues with the next pin regardless.
    fn undo_pin(&mut self, pin: u8) -> Result<(), Self::Error>;
}

/// Applies the wake configuration of `plan` through `ops`, rolling back on failure.
///
/// # Errors
///
/// The error of the first failing apply step, after the rollback has run.
pub fn arm_transactionally<O: WakeArmOps>(ops: &mut O, plan: SleepPlan) -> Result<(), O::Error> {
    let result = apply(ops, plan);
    if result.is_err() {
        roll_back(ops);
    }
    result
}

fn apply<O: WakeArmOps>(ops: &mut O, plan: SleepPlan) -> Result<(), O::Error> {
    ops.clear(plan.pin_mask)?;
    if let Some(duration_ms) = plan.timer {
        ops.arm_timer(duration_ms)?;
    }
    if plan.has_pins() {
        ops.arm_pins(plan.pin_mask, plan.level)?;
    }
    ops.finish()
}

fn roll_back<O: WakeArmOps>(ops: &mut O) {
    // Each step is attempted whatever the previous one returned; the tier logs failures.
    let _ = ops.undo_sources();
    let armed = ops.armed_pins();
    for pin in 0..64u8 {
        if armed & (1u64 << pin) != 0 {
            let _ = ops.undo_pin(pin);
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use std::vec::Vec;

    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Op {
        Clear(u64),
        ArmTimer(u64),
        ArmPins(u64, GpioWakeLevel),
        Finish,
        UndoSources,
        ArmedPins,
        UndoPin(u8),
    }

    /// Records every operation and keeps the per-pin wake enables like the C2/C3 register.
    struct FakeOps {
        calls: Vec<Op>,
        fail: Vec<Op>,
        armed: u64,
        timer_armed: bool,
    }

    impl FakeOps {
        fn new(armed: u64) -> Self {
            Self {
                calls: Vec::new(),
                fail: Vec::new(),
                armed,
                timer_armed: false,
            }
        }

        fn step(&mut self, op: Op) -> Result<(), Op> {
            self.calls.push(op);
            if self.fail.contains(&op) {
                Err(op)
            } else {
                Ok(())
            }
        }
    }

    impl WakeArmOps for FakeOps {
        type Error = Op;

        fn clear(&mut self, keep_pins: u64) -> Result<(), Op> {
            self.step(Op::Clear(keep_pins))?;
            self.armed &= keep_pins;
            self.timer_armed = false;
            Ok(())
        }

        fn arm_timer(&mut self, duration_ms: u64) -> Result<(), Op> {
            self.step(Op::ArmTimer(duration_ms))?;
            self.timer_armed = true;
            Ok(())
        }

        fn arm_pins(&mut self, pin_mask: u64, level: GpioWakeLevel) -> Result<(), Op> {
            // Like the SDK loop: pins armed before the failing one stay armed.
            self.armed |= pin_mask & pin_mask.wrapping_neg();
            self.step(Op::ArmPins(pin_mask, level))?;
            self.armed |= pin_mask;
            Ok(())
        }

        fn finish(&mut self) -> Result<(), Op> {
            self.step(Op::Finish)
        }

        fn undo_sources(&mut self) -> Result<(), Op> {
            self.step(Op::UndoSources)?;
            self.timer_armed = false;
            Ok(())
        }

        fn armed_pins(&mut self) -> u64 {
            self.calls.push(Op::ArmedPins);
            self.armed
        }

        fn undo_pin(&mut self, pin: u8) -> Result<(), Op> {
            self.step(Op::UndoPin(pin))?;
            self.armed &= !(1u64 << pin);
            Ok(())
        }
    }

    const PINS: u64 = 0b1010; // GPIO1 and GPIO3

    fn plan(timer: Option<u64>, pin_mask: u64) -> SleepPlan {
        SleepPlan {
            timer,
            pin_mask,
            level: GpioWakeLevel::AnyHigh,
        }
    }

    #[test]
    fn success_applies_in_order_without_rollback() {
        let mut ops = FakeOps::new(0);
        assert_eq!(arm_transactionally(&mut ops, plan(Some(500), PINS)), Ok(()));
        assert_eq!(
            ops.calls,
            [
                Op::Clear(PINS),
                Op::ArmTimer(500),
                Op::ArmPins(PINS, GpioWakeLevel::AnyHigh),
                Op::Finish,
            ]
        );
        assert!(ops.timer_armed);
        assert_eq!(ops.armed, PINS);
    }

    #[test]
    fn gpio_failure_after_timer_rolls_everything_back_and_keeps_the_error() {
        let mut ops = FakeOps::new(0);
        ops.fail.push(Op::ArmPins(PINS, GpioWakeLevel::AnyHigh));
        assert_eq!(
            arm_transactionally(&mut ops, plan(Some(500), PINS)),
            Err(Op::ArmPins(PINS, GpioWakeLevel::AnyHigh))
        );
        assert_eq!(
            ops.calls,
            [
                Op::Clear(PINS),
                Op::ArmTimer(500),
                Op::ArmPins(PINS, GpioWakeLevel::AnyHigh),
                Op::UndoSources,
                Op::ArmedPins,
                Op::UndoPin(1),
            ]
        );
        assert!(!ops.timer_armed);
        assert_eq!(ops.armed, 0);
    }

    #[test]
    fn rollback_continues_after_failing_cleanup_steps() {
        // Stale pins 0 and 5 survive a failing clear; pin 0's undo fails too.
        let mut ops = FakeOps::new(0b10_0001);
        ops.fail.push(Op::Clear(PINS));
        ops.fail.push(Op::UndoSources);
        ops.fail.push(Op::UndoPin(0));
        assert_eq!(
            arm_transactionally(&mut ops, plan(None, PINS)),
            Err(Op::Clear(PINS))
        );
        assert_eq!(
            ops.calls,
            [
                Op::Clear(PINS),
                Op::UndoSources,
                Op::ArmedPins,
                Op::UndoPin(0),
                Op::UndoPin(5),
            ]
        );
        assert_eq!(
            ops.armed, 0b00_0001,
            "only the pin whose undo failed stays armed"
        );
    }

    #[test]
    fn finish_failure_rolls_back() {
        let mut ops = FakeOps::new(0);
        ops.fail.push(Op::Finish);
        assert_eq!(
            arm_transactionally(&mut ops, plan(Some(500), PINS)),
            Err(Op::Finish)
        );
        assert!(!ops.timer_armed);
        assert_eq!(ops.armed, 0);
    }

    #[test]
    fn timer_only_plan_never_arms_pins() {
        let mut ops = FakeOps::new(0);
        ops.fail.push(Op::ArmTimer(500));
        assert_eq!(
            arm_transactionally(&mut ops, plan(Some(500), 0)),
            Err(Op::ArmTimer(500))
        );
        assert_eq!(
            ops.calls,
            [
                Op::Clear(0),
                Op::ArmTimer(500),
                Op::UndoSources,
                Op::ArmedPins
            ]
        );
    }
}
