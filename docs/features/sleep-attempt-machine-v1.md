---
gate: none
desk-work: available
---

# Feature: Sleep Attempt Machine v1

A sans-IO `SleepAttemptMachine` in `stoker::sleep::attempt` owns the `try_sleep` sequence; both tiers only perform its actions.
The pattern and its rationale are in [ADR-004](../adr/004-shared-logic-ladder.md); the postcondition it preserves is [ADR-003](../adr/003-tier-parity-contract.md) point 1.

## Guard table (`ChipPolicy::guard`)
| Chip | Pre-read of the wake pins | Reason |
|:--|:--|:--|
| C2, C3 | `Always` | The chip sleeps on a held pin and boot-loops |
| ESP32 | `Always` | The chip never rejects on EXT1 |
| S3 | `WhenHoldEffective` | A rejection after digital-pad isolation hangs; the hardware rejects otherwise |
| C6, Other | `Never` | Hardware reject assumed; unproven on the C6 |

## Decisions
| Decision | Reason | Rejected Alternative |
|---:|:--|:--|
| Machine owns the sequence order; tiers own register I/O | Step order and guards are host-testable | Pure decision fns only |
| One `ChipPolicy` guard table shared by both tiers | Closes guard drift between tiers | Tier-supplied policy |
| `SleepPlan` extracted once in `new` | Removes the per-tier plan and level copies | Re-extract per step |
| Multi-pin `AnyLow` on the ESP32 refused in `SleepAttemptMachine::new` | The chip has only `ALL_LOW` (ADR-004 point 7) | Per-tier checks |
| `average_mv` and `classify_charging` as pure `stoker` fns | Flat logic; a machine would be over-engineering | Machines for ADC and charging |
| The C2 policy row is compile-only | No `just` recipe targets the C2 | A C2 bench |

## Constraints
- `no_std`, no allocation; inputs, actions and the machine are `Copy`-sized values.
- On esp-hal the whole poll loop stays inside the existing `critical_section::with`; the pin read adds no UART drain time.
- Pin levels are read by the tier as late as possible and fed in as `PinLevels`.
- A best-effort release never changes the `Rejected` kind on the IDF tier.
- A tier answers `Released` only after undoing its arming, so wake pins are usable as plain inputs (ADR-003 point 1).
- The release mechanism (`rtc_gpio_*` cfg forks, `Input` handles) stays in the tiers.
- The machine is not cfg-gated per chip; `ChipPolicy` is a runtime value the tier passes in.
