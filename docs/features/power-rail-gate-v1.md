---
gate: none
desk-work: available
---

# Feature: Power Rail Gate v1

A `PowerRailGate` trait in `stoker` with a host-testable sleep coordinator and GPIO-driven implementations in both tiers, so `rustyfarian-network` can switch a rail off between uplinks.
Brainstormed as "RadioPowerGate" (B1–B3 of the cross-repo plan); B4 (network side) lives in the network repos.
Verified 2026-10-24 (Zephyr board DTS): on the Heltec V3 the SX1262 has no power domain, only Vext is switchable (GPIO36, active-low, pulled up) and it feeds the OLED.

## Decisions
|                                                                                   Decision | Reason                                                        | Rejected Alternative                              |
|-------------------------------------------------------------------------------------------:|:--------------------------------------------------------------|:--------------------------------------------------|
|                              Trait `PowerRailGate` in `stoker::gate`, not `RadioPowerGate` | Heltec V3 radio is on always-on 3V3; the gate switches a rail | Keep the `RadioPowerGate` name from README/VISION |
|             `power_on`, `power_off`, `is_powered`; `type Error: gate::Error` with `kind()` | ADR-001 shape; a rail that refuses is an operational failure  | Infallible `()` returns                           |
|                                           `NoopPowerRailGate` mock with injectable failure | Host tests for consumers and the coordinator                  | —                                                 |
| Coordinator in `stoker` wraps a gate and a `SleepManager`: off before sleep, on after wake | Host-testable sequencing; `SleepManager` untouched            | `EspSleepManager` takes an optional gate          |
|                  Generic `GpioRailGate<P: OutputPin>` with polarity, defined once per tier | `stoker` keeps `critical-section` as its only dependency      | Optional `embedded-hal` feature in `stoker`       |
|                                                 Heltec V3 preset: Vext, GPIO36, active-low | DTS: OLED on `vext_ctrl`, SX1262 on no power domain           | "GPIO3 gates the radio" (old key-insights, wrong) |
|                               SX1262 `SetSleep` over SPI stays in the network radio driver | Power never imports network; the gate is a rail primitive     | Radio-sleep hook on the trait                     |
|                                                    No settle delay in trait or coordinator | Load-dependent; the consumer delays after `power_on`          | Off → Powering → Ready state machine              |
|                                                    Ships in the 0.3.0 `[Unreleased]` train | One CHANGELOG cut with the error convention it builds on      | Separate 0.4.0                                    |
|                                ADR-002 records the trait shape, error type and coordinator | Cross-repo contract consumed by `rustyfarian-network`         | —                                                 |
|    Coordinator `GatedSleep<G, S>` owns gate and sleep manager, `into_parts()` returns them | Simple lifetimes across a sleep call                          | Borrowed `&mut` view                              |
|                                              `is_powered` returns cached state, infallible | `OutputPin` cannot read itself back on every HAL              | `StatefulOutputPin` read-back                     |
|                                                     `&mut self` receivers, no `Send` bound | Mirrors `SleepManager` and `BatteryMonitor`                   | `&self` with interior mutability                  |

## Constraints
- `stoker` stays `#![no_std]` with `critical-section` as its only dependency.
- Both tiers implement the gate over an `embedded_hal::digital::OutputPin` with configurable polarity.
- `Vext_Ctrl` is pulled up, so the rail is off unless the pin is actively driven low; a released pad in deep sleep means off.
- The Heltec V3 cannot gate the SX1262; the bench measures the Vext load (OLED, Ve header), not the radio.
- Dependencies flow application → network → power; the gate carries no network types.
- The network-side acceptance of the trait (B4) is out of scope for this repo.

## Open Questions
- [ ] Does the GPIO36 pull-up keep Vext off through deep sleep on IDF and esp-hal, or does the pad need hold or isolation? Bench on the V3.
- [ ] How much does Vext off save on the V3 with the OLED idle versus `displayOff`? Decides whether the bench number is meaningful.
- [x] Coordinator owns both by value (2026-10-24); the esp-hal async sleep path is a later addition.
- [x] Network takes `&mut impl PowerRailGate`; `Send` is the consumer's choice (2026-10-24).
- [x] `is_powered` returns cached state (2026-10-24).

## State
- [x] Design approved (2026-10-24 dialog)
- [x] ADR-002 written and accepted
- [x] `stoker`: trait, error type, `NoopPowerRailGate`, coordinator, host tests (double on, off while off, power-on failure, sleep sequencing)
- [x] IDF tier: `GpioRailGate`, Heltec V3 Vext preset, example
- [x] esp-hal tier: `GpioRailGate`, Heltec V3 Vext preset, example
- [ ] Heltec V3 bench: Vext current off vs on, rail off through deep sleep
- [x] Tests passing (`just verify`, `just check-all`, `just check-hal`, both S3 example builds, 2026-10-24)
- [x] Documentation: CHANGELOG, README family diagram (planned → wired), VISION wording, key-insights

## Session Log
- 2026-10-24 — Feature doc created via /feature dialog; Vext vs SX1262 rail verified, trait renamed to `PowerRailGate`
- 2026-10-24 — Updated: bench gate cleared (V3 and meter ready), coordinator ownership, `is_powered` and receiver decided; implementation starts
- 2026-10-24 — Implemented: stoker gate module, both tiers, ADR-002, docs; bench pending
