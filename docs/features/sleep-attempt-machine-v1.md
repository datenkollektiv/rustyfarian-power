---
gate: none
desk-work: available
---

# Feature: Sleep Attempt Machine v1

A sans-IO `SleepAttemptMachine` in `stoker::sleep::attempt` owns the `try_sleep` sequence; both tiers only perform its actions.
Pattern copied from `rustyfarian-network` ADR-022 decision 10 (`LinkMachine`, `SessionMachine`); rationale in [ADR-004](../adr/004-shared-logic-ladder.md).

## Decisions
|                                                             Decision | Reason                                              | Rejected Alternative                          |
|---------------------------------------------------------------------:|:----------------------------------------------------|:----------------------------------------------|
| Sans-IO machine: `SleepAttemptInput` → `poll` → `SleepAttemptAction` | Same shape as the network siblings                  | Tier-owned dispatch loops (ADR-004 alternatives) |
|                  Machine owns sequence order; tiers own register I/O | Step order and guards host-testable                 | Pure decision fns only (tiers keep order)     |
|                    One `ChipPolicy` guard table shared by both tiers | Closes guard drift between tiers                    | Tier-supplied policy (byte-for-byte refactor) |
|                            C2/C3 and ESP32 always pre-read wake pins | C3 boot-loop; ESP32 never rejects on EXT1           | Leave ESP32 divergence open                   |
|                  S3 pre-reads only when global pad hold is effective | Matches bench R2/R3-S3 findings                     | Always pre-read on S3                         |
|                                  `SleepPlan` extracted once in `new` | Removes 3× IDF and 2× hal plan/level copies         | Re-extract per step                           |
|            `average_mv` and `classify_charging` as pure `stoker` fns | Flat logic; a machine would be over-engineering     | Machines for ADC and charging                 |
|                         ADR-004 partially supersedes ADR-003 point 3 | Sequencing moves into `stoker` for multi-step flows | Amend ADR-003 in place                        |

## Constraints
- `no_std`, no allocation; inputs, actions and the machine are `Copy`-sized values.
- On esp-hal the whole poll loop stays inside the existing `critical_section::with`.
- Pin levels are read by the tier as late as possible and fed in as `PinLevels`.
- A best-effort release never changes the `Rejected` kind on the IDF tier.
- A tier answers `Released` only after undoing its arming and making the ADR-003 point 1 postcondition hold.
- ADR-003 point 1 postcondition is preserved: wake pins usable as plain inputs after `Rejected`.
- Release mechanism (`rtc_gpio_*` cfg forks, `Input` handles) stays in the tiers.
- Machine must not be cfg-gated per chip; `ChipPolicy` is a runtime value the tier passes in.
- Conformance modules keep compiling on every chip recipe.

## Open Questions
- [x] Does the hal C3 guard need a timing check so the critical section does not starve the UART drain?
  - Answer (esp-hal tier, 2026-11-04): no new timing check is added.
  - The UART drain already runs inside `arm()` in the critical section before this change.
  - The pin read happens after `Armed` as register reads, so it adds no drain time.
  - The drain stays bounded by `DRAIN_US` (provisional); the R2 bench re-run confirms the UART capture.
- [x] Is the C2 covered by any recipe, or is its policy row compile-only?
  - Answer: no `just` recipe targets C2.
  - The C2 guard is reached only through the IDF tier's C2/C3 cfg, so it is compile-only and has no bench.

## State
- [x] Design approved
- [x] ADR-004 written
- [x] `stoker` machine, `ChipPolicy`, `SleepPlan` with host tests
- [x] `average_mv` and `classify_charging` with host tests; IDF tier uses them
- [x] IDF tier drives the machine
- [x] esp-hal tier drives the machine (gains C3 guard; ESP32 and C6 sleep modules added 2026-11-04)
- [x] Multi-pin `AnyLow` on the ESP32 is a `stoker` check in `SleepAttemptMachine::new` (`ChipPolicy::any_low_multi_pin`), on both tiers (ADR-004 point 7)
- [x] Shared `drive` loop over an I/O-only `SleepAttemptPort`; both tiers implement the port, with host tests per terminal path
- [ ] C6 guard `Never` is provisional until the C6 hardware pass (see `idf-c6-tier-v1.md`)
- [x] `just verify`, `just check-all`, chip recipes and example builds green
- Hardware validation: a single final pass follows the tier-parity changes, before the release; it is not a gate for this feature (benches R2–R4 on C3 and S3, both tiers)
- [x] Capability table, key-insights and CHANGELOG updated

## Session Log
- 2026-11-04 — Feature doc created via /feature dialog after brainstorming against rustyfarian-network's sans-IO cores
- 2026-11-04 — Machine landed in `stoker::sleep::attempt`; IDF and esp-hal tiers drive it; ESP32 and hal C3 guards built, benches R2–R4 pending
- 2026-11-04 — Hardware gate removed; benches move to the final pre-release hardware pass
- 2026-11-04 — Shared `drive` over `SleepAttemptPort`, ESP32 multi-pin `AnyLow` rejected on both tiers, C6 guard marked provisional; benches still pending
