---
gate: none
desk-work: available
---

# Feature: Power Rail Gate v1

A `PowerRailGate` trait in `stoker` with a host-testable sleep coordinator and a generic GPIO gate, so `rustyfarian-network` can switch a rail off between uplinks.
Trait shape, error type, coordinator and what the network may rely on are in [ADR-002](../adr/002-power-rail-gate-contract.md); the network side (B4) lives in the network repos.

## Decisions
| Decision | Reason | Rejected Alternative |
|---:|:--|:--|
| Trait `PowerRailGate`, not `RadioPowerGate` | The Heltec V3 radio is on always-on 3V3; only Vext (GPIO36, active-low) switches | Keep the `RadioPowerGate` name |
| `GpioRailGate<P: OutputPin>` with polarity, defined once in `stoker` | One gate for both tiers (ADR-004 rung 1) | Per-tier copies |
| `NoopPowerRailGate` mock with injectable failure | Host tests for consumers and the coordinator | — |
| Heltec V3 preset: Vext, GPIO36, active-low, per tier | DTS: OLED on `vext_ctrl`, SX1262 on no power domain | "GPIO3 gates the radio" |
| `&mut self` receivers, no `Send` bound | Mirrors `SleepManager` and `BatteryMonitor` | `&self` with interior mutability |

## Constraints
- `stoker` stays `#![no_std]` and alloc-free; it depends on `critical-section` and `embedded-hal` 1.0.
- `Vext_Ctrl` must stay driven through deep sleep: a released pad turns the rail on, so the IDF preset keeps GPIO36 out of the sleep-time GPIO isolation and esp-hal keeps the output latch.
- The Heltec V3 cannot gate the SX1262; the bench measures the Vext load (OLED, Ve header), not the radio.
- Dependencies flow from application to network to power; the gate carries no network types.

## State
- [ ] How much Vext off saves on the V3 with the OLED idle versus `displayOff`; decides whether a bench current figure is meaningful.
