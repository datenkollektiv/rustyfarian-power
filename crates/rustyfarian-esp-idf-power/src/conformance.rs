//! Compile-time conformance of the ESP-IDF adapters to the `stoker` traits (ADR-003 point 4).
//!
//! Each `assert_*` function is generic over a `stoker` trait and calls every method of it,
//! and each `instantiate` function instantiates them with this tier's concrete types.
//! A change to a `stoker` trait (renamed or re-typed method, new required item) therefore
//! breaks this crate's build instead of drifting silently.
//! The module is private and never called; it proves API shape only, not behaviour (benches do).

#![allow(dead_code)]

use esp_idf_hal::adc::{AdcChannel, ADCU1};
use esp_idf_hal::gpio::{ADCPin, Output, PinDriver};
use stoker::gate::Error as GateErrorTrait;
use stoker::sleep::Error as SleepErrorTrait;
use stoker::{
    BatteryMonitor, ChargingMonitor, PowerRailGate, ResetReasonSource, SleepManager,
    WakeCauseSource, WakeSource,
};

use crate::{
    EspAdcBatteryMonitor, EspChargingMonitor, EspResetReasonSource, EspSleepManager,
    EspWakeCauseSource, GpioRailGate,
};

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

/// Calls every [`BatteryMonitor`] method.
fn assert_battery_monitor<M: BatteryMonitor>(monitor: &mut M) {
    let _ = monitor.read();
}

/// Calls every [`ChargingMonitor`] method.
fn assert_charging_monitor<M: ChargingMonitor>(monitor: &mut M) {
    let _ = monitor.read_charging_state();
}

/// Instantiates the generic checks with the concrete ESP-IDF adapter types.
fn instantiate_sleep(manager: &mut EspSleepManager, sources: &[WakeSource]) {
    assert_sleep_manager::<EspSleepManager>(manager, sources);
}

fn instantiate_wake_cause(source: &EspWakeCauseSource) {
    assert_wake_cause_source::<EspWakeCauseSource>(source);
}

fn instantiate_reset_reason(source: &EspResetReasonSource) {
    assert_reset_reason_source::<EspResetReasonSource>(source);
}

fn instantiate_rail_gate(gate: &mut GpioRailGate<PinDriver<'static, Output>>) {
    assert_power_rail_gate::<GpioRailGate<PinDriver<'static, Output>>>(gate);
}

/// Generic in the ADC pin: the monitor must implement the trait for every ADC1 pin.
fn instantiate_battery_monitor<'a, GPIO>(monitor: &mut EspAdcBatteryMonitor<'a, GPIO>)
where
    GPIO: ADCPin + 'a,
    GPIO::AdcChannel: AdcChannel<AdcUnit = ADCU1>,
{
    assert_battery_monitor::<EspAdcBatteryMonitor<'a, GPIO>>(monitor);
}

fn instantiate_charging_monitor(monitor: &mut EspChargingMonitor<'static>) {
    assert_charging_monitor::<EspChargingMonitor<'static>>(monitor);
}
