# ADR-004: Shared tier logic follows a three-rung ladder

## Status
Accepted (2026-11-04).
Partially supersedes ADR-003 point 3 for multi-step flows; register work stays in the tier adapters.
Adapts `rustyfarian-network` ADR-022 decision 10; `rustyfarian-peripherals` ADR-010 adopts the same ladder.
Platform pins follow ADR-001.

## Context
ADR-003 point 3 places all sequencing in the tier adapters, because the steps touch hardware.
Where both tiers sequence the same flow, each re-implements the order of the steps, and the two copies drift.
`try_sleep` was the first flow where that drift produced a cross-tier gap: the ESP-IDF tier pre-read wake pins on the C3, the esp-hal tier did not.
`rustyfarian-network` ADR-022 decision 10 answers the same drift with sans-IO state machines (`LinkMachine`, `SessionMachine`): inputs go in, actions come out, and a thin loop per tier performs them.
Not every shared flow needs a machine.
Where `embedded-hal` 1.0 covers the I/O, a driver generic over its traits is simpler and has no second copy of the order.
Much of `stoker` is flat logic (`voltage_to_percent`, `validate_wake_sources`), where a machine would be ceremony.

## Decision
1. Shared logic takes the lowest rung that fits:
   - **Rung 1 — generic driver.** The flow lives in `stoker`, generic over `embedded-hal` traits. Use this when the traits cover the I/O.
   - **Rung 2 — event machine.** The core decodes samples: `update(sample, now) -> Option<Event>`. A tier, an ISR, or an `embedded-hal` adapter feeds it.
   - **Rung 3 — action machine.** The core emits I/O requests: `poll(&mut self, input) -> Action`, and a thin loop per tier performs them.
2. Rung 3 is used only when a flow meets one of these triggers:
   - It must run under more than one execution model, such as blocking, async, or a cooperative tick loop.
   - The tiers perform I/O outside `embedded-hal`, such as register access or sleep entry, but must share the order of the steps.
3. Flat logic stays as pure functions in `stoker` (`average_mv`, `classify_charging`, `voltage_to_percent`), not machines.
4. Time is an input: a machine never reads a clock; the caller passes `now` or reports elapsed waits.
5. A rung-3 machine answers out-of-phase input with an explicit no-op action, so a host test of a tier loop can detect it.
6. Every rung-3 flow has a finite transition path to a terminal action; the tier loop exits on a protocol violation (a no-op action), and hardware waits need their own bounds.
   Cancellation and abandoned-attempt cleanup are defined when an async or cooperative runner appears.
7. Shared behavioural decisions and ordering belong in `stoker`; branches that implement a hardware operation, validate adapter ownership, or translate platform results stay in the adapter.
8. Per-chip behaviour is a runtime value passed in by the tier, not a `cfg` gate inside the machine.
9. Tier-specific facts stay in the adapters: register access, critical sections, late pin reads, sleep entry, and the cfg-gated release mechanism.
10. ADR-003 point 3 is partially superseded: sequencing for flows on rung 2 or 3 moves into `stoker`; the rest of point 3 stands.

## First instance
`try_sleep` is rung 3 under the second trigger: its I/O is register access and sleep entry, which `embedded-hal` does not cover.
`SleepAttemptMachine` in `stoker::sleep::attempt` owns the order of the steps, and `ChipPolicy` holds one guard table for both tiers.
The guard table, the adapter contract, and the bench requirements are in `docs/features/sleep-attempt-machine-v1.md`.

`GpioRailGate` (one `OutputPin`) is the first rung-1 instance: it lives in `stoker::gate`, and both tiers re-export the one type.
The orphan rule forbids an inherent `heltec_v3_vext` on the foreign generic, so each tier exports a local extension trait `HeltecV3Vext`; call sites stay `GpioRailGate::heltec_v3_vext(pin)` after a `use`.
`GpioChargingMonitor` (STAT and VBUS `InputPin`s) is the second rung-1 instance: it lives in `stoker::charging`, the IDF `EspChargingMonitor` wraps it, and the esp-hal tier gains `EspHalChargingMonitor` as an alias.
The extension trait was chosen over a free function because it keeps the call shape `GpioRailGate::heltec_v3_vext(pin)`; callers add `HeltecV3Vext` to their imports, which is the one breaking part.
No rung-1 candidate remains unmoved; ADC sampling stays tier I/O plus `stoker::average_mv`, because `embedded-hal` 1.0 has no ADC trait.
`stoker` therefore depends on `embedded-hal` 1.0 (default features), and stays `no_std` and allocation-free.
The per-chip wake-pin mask moved into `ChipPolicy::wake_pin_mask()`, the single chip table (point 8); `SleepAttemptMachine::new` range-checks against it, so neither tier keeps a per-chip `cfg` mask.
Amended 2026-11-04: the IDF C6 build maps to `ChipPolicy::Esp32C6` (guard `Never`, mask `0xFF`), as a tracked tier-parity work item; before that it stayed `Other` (mask check skipped).
The C6 guard `Never` is provisional: it is assumed to reject on its own and is unproven on hardware.
The C6 hardware pass judges both the `try_sleep` result and the next boot's reset and wake cause.
The per-chip row is selected by `cfg(esp32c6)` in the IDF tier and by the `CHIP_POLICY` constant in the hal tier.
No rung-2 instance exists today.

## Consequences
Positive:
- Shared step orders are host-testable in `stoker`, and both tiers run the same order.
- Adapters shrink to I/O and error mapping, which the review rule can check.
- The ladder keeps machines out of flows that a generic driver or a pure function covers.

Negative:
- Rung 3 adds one indirection: a reader follows an action from `stoker` into the tier loop.
- Host tests prove the order, not the hardware; the ADR-001 benches remain the arbiter.
- Constraints on the tier loop, such as running inside a critical section or reading a pin late, are not checked by any type.

Constraints:
- Machines are `no_std`, allocation-free, and free of async; inputs, actions and the machine are small `Copy` values.
- Rung 2 and 3 machines need phase-transition tests in `stoker`.
- Moving a flow up a rung is demand-driven: the first change that needs it pays for it.
- A change to a rung-3 machine re-runs the benches for the flow it drives (for `try_sleep`: R2 to R4 on C3 and S3, both tiers); the ladder rollout of 2026-11-04 defers them to one final hardware pass before the release.

## Alternatives Considered
|                  Alternative | Pros                              | Cons                                                       | Why Rejected                                                               |
|-----------------------------:|:----------------------------------|:-----------------------------------------------------------|:---------------------------------------------------------------------------|
|       Register-owning runner | One shared loop with explicit I/O | Port must model every register step outside `embedded-hal` | Pulls register steps into `stoker`; adopted `drive` takes an I/O-only port |
|   Action machines everywhere | One uniform shape                 | Ceremony for flat logic and `embedded-hal` drivers         | No drift to fix; adds indirection for nothing                              |
| Pure decision functions only | Host-testable, no new types       | Tiers keep the step order, so the order can still drift    | The order is the drift source; functions cannot pin it                     |
|        Tier-owned sequencing | Each tier tunes its own flow      | Two copies of the order drift apart                        | The failure mode that `try_sleep` hit                                      |
