---
gate: none
desk-work: available
---

# Feature: esp-hal Parity (ADC + Charging) v1

New `rustyfarian-esp-hal-power` crate gives the bare-metal tier battery monitoring and charging detection, matching the esp-idf tier.
Deep sleep and wake cause are out of scope here; they need a `stoker` trait redesign (ADR first) and get their own v2 doc.
Capability findings: `docs/key-insights.md` § esp-hal tier.

## Decisions
|                                      Decision | Reason                                             | Rejected Alternative              |
|----------------------------------------------:|:---------------------------------------------------|:----------------------------------|
| Stage: ADC + charging in v1, sleep/wake in v2 | Sleep contract is IDF-shaped and needs an ADR      | Full parity in one feature        |
|         New crate `rustyfarian-esp-hal-power` | Matches ws2812/network/peripherals tier layout     | Feature flag on the IDF crate     |
| `stoker` becomes `#![no_std]` unconditionally | One API shape for both tiers                       | `std` feature keeping `anyhow`    |
|   Replace `anyhow` with a `stoker` error enum | `anyhow` blocks no_std; IDF crate maps at its edge | Associated `type Error` per trait |
|    Chips: C3, C6, ESP32, S3 via chip features | Same four chip features as the sibling hal crates  | ESP32 + S3 only                   |
|          Blocking API plus an `async` feature | Mirrors ws2812/network (esp-rtos + embassy)        | Blocking only                     |

## Constraints
- `esp-*` crates are exact-pinned (`esp-hal =1.2.2`) in lockstep with the sibling repos.
- Embassy pins match the siblings exactly (`embassy-executor =0.10.0`, `embassy-sync =0.8.0`, `embassy-time =0.5.1`).
- The hal crate declares MSRV 1.95; `stoker` keeps MSRV 1.88.
- The esp-idf tier must not regress: `just check-all` and `just check-esp32` stay green.
- Hal builds use a separate `target/hal` dir so they do not thrash the IDF cache.
- `stoker`, `rustyfarian-esp-idf-power` and the hal crate release together as 0.3.0 (breaking: error type).
- v1 is not complete until battery examples run on Heltec V3 (S3) and Feather V2 (ESP32).

## Open Questions
- [ ] Is async a separate `AsyncBatteryMonitor` trait in `stoker`, or inherent async methods on the hal driver, and does esp-hal 1.2.2 offer async ADC at all?
- [ ] Which battery-equipped C3/C6 boards get `BatteryConfig` presets and hardware validation, or are they compile-only?
- [ ] Which esp-hal calibration scheme per chip matches the IDF tier's mV readings within tolerance?
- [ ] How should `stoker`'s `build.rs` derive the wake-pin mask for `*-none-elf` triples (it currently falls through to the S3 mask)?

## State
- [x] Design approved
- [ ] `stoker` no_std + error enum (IDF crate migrated)
- [ ] `rustyfarian-esp-hal-power` scaffold, `just` hal recipes, `target/hal`
- [ ] ADC battery monitor + charging monitor (blocking)
- [ ] `async` feature
- [ ] Examples build for all four chips
- [ ] Hardware-validated on Heltec V3 and Feather V2
- [ ] Tests passing (`just verify`)
- [ ] Documentation updated (READMEs, key-insights, CHANGELOG)

## Session Log
- 2026-09-26 — Feature doc created via /feature dialog
