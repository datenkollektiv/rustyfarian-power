//! Compile-time conformance of the esp-hal adapters to the `stoker` traits (ADR-003 point 4).
//!
//! Each `assert_*` function is generic over a `stoker` trait and calls every method of it,
//! and the `instantiate_*` functions instantiate them with this tier's concrete types,
//! `BoundSleep` with all four lifetimes named.
//! A change to a `stoker` trait (renamed or re-typed method, new required item) therefore
//! breaks this crate's build instead of drifting silently.
//! The module is private and never called; it proves API shape only, not behaviour (benches do).
//!
//! The sleep, wake-cause and reset checks carry the chip cfg of the types they cover
//! (ESP32-C3 and ESP32-S3); the rail gate check applies to every chip.

#![allow(dead_code)]

use esp_hal::gpio::Output;
use stoker::gate::Error as GateErrorTrait;
use stoker::PowerRailGate;

use crate::GpioRailGate;

/// Calls every [`PowerRailGate`] method and its error classification.
fn assert_power_rail_gate<G>(gate: &mut G)
where
    G: PowerRailGate,
    G::Error: GateErrorTrait,
{
    if let Err(e) = gate.power_on() {
        let _ = e.kind();
    }
    if let Err(e) = gate.power_off() {
        let _ = e.kind();
    }
}

fn instantiate_rail_gate(gate: &mut GpioRailGate<Output<'static>>) {
    assert_power_rail_gate::<GpioRailGate<Output<'static>>>(gate);
}

#[cfg(any(feature = "esp32c3", feature = "esp32s3"))]
mod chip {
    use esp_hal::gpio::{Input, Level};
    use stoker::sleep::Error as SleepErrorTrait;
    use stoker::{ResetReasonSource, SleepManager, WakeCauseSource, WakeSource};

    use crate::{BoundSleep, EspHalResetReasonSource, EspHalSleepManager, EspHalWakeCauseSource};

    /// Calls every [`SleepManager`] method and its error classification.
    fn assert_sleep_manager<M>(manager: &mut M, sources: &[WakeSource])
    where
        M: SleepManager,
        M::Error: SleepErrorTrait,
    {
        if let Err(e) = manager.sleep(sources) {
            let _ = e.kind();
        }
        if let Err(e) = manager.try_sleep(sources) {
            let _ = e.kind();
        }
    }

    /// Calls every [`WakeCauseSource`] method.
    fn assert_wake_cause_source<S: WakeCauseSource>(source: &S) {
        let _ = source.last_wake_cause();
    }

    /// Calls every [`ResetReasonSource`] method.
    fn assert_reset_reason_source<S: ResetReasonSource>(source: &S) {
        let _ = source.last_reset_reason();
    }

    /// `BoundSleep` with every lifetime named, so a change to its lifetime shape breaks the build.
    fn instantiate_bound_sleep<'a, 'p, 'i, 'd>(
        bound: &mut BoundSleep<'a, 'p, 'i, 'd>,
        sources: &[WakeSource],
    ) {
        assert_sleep_manager::<BoundSleep<'a, 'p, 'i, 'd>>(bound, sources);
    }

    /// The real per-attempt shape: bind the wake pin, sleep through the generic trait, and read the
    /// same pin again once the `BoundSleep` borrow has ended.
    fn bind_try_sleep_then_read<'d, 'i>(
        manager: &mut EspHalSleepManager<'d>,
        pin: &mut Input<'i>,
        sources: &[WakeSource],
    ) -> Level {
        {
            let mut pins = [&mut *pin];
            let mut bound = manager.bind_wake_pins(&mut pins);
            assert_sleep_manager(&mut bound, sources);
        }
        pin.level()
    }

    fn instantiate_wake_cause(source: &EspHalWakeCauseSource) {
        assert_wake_cause_source::<EspHalWakeCauseSource>(source);
    }

    fn instantiate_reset_reason(source: &EspHalResetReasonSource) {
        assert_reset_reason_source::<EspHalResetReasonSource>(source);
    }
}
