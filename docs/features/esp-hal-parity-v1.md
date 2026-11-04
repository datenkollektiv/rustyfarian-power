---
gate: none
desk-work: available
---

# Feature: esp-hal Parity (ADC + Charging) v1

New `rustyfarian-esp-hal-power` crate gives the bare-metal tier battery monitoring and charging detection, matching the esp-idf tier.
Deep sleep and wake cause are out of scope here; their `stoker` trait redesign is [ADR-001](../adr/001-hal-neutral-sleep-contract.md) and they live in `esp-hal-sleep-tier-v1.md`, which now lands first in the hal crate.
Capability findings: `docs/key-insights.md` § esp-hal tier.

## Capability table (ADR-003 point 2)

One row per shared capability, per tier and chip, as of 2026-11-04.
Status: **bench** = hardware-validated, **build** = compiled and linked, **compile** = compile-only, **—** = not built.
Class (ADR-003 point 2): **HW** hardware limitation, **UD** unimplemented driver, **BD** behavioural difference; blank = same observables on every built cell.
The esp-hal tier targets C3, S3, ESP32 and C6; the ESP-IDF tier targets C3, S3 and ESP32 (no C6 recipe).

| Capability (`stoker` trait or type)                | IDF C3         | IDF S3         | IDF ESP32                              | hal C3     | hal S3        | hal ESP32 | hal C6  | Class and note                                                                                                                                        |
|:---------------------------------------------------|:---------------|:---------------|:---------------------------------------|:-----------|:--------------|:----------|:--------|:------------------------------------------------------------------------------------------------------------------------------------------------------|
| `SleepManager::sleep` / `try_sleep`, GPIO + timer  | bench (R2)     | bench (R2-R4)  | timer bench (Feather V2), GPIO compile | bench (R2) | bench (R2-R4) | —         | —       | UD on hal ESP32/C6 (no sleep module). BD on IDF ESP32: the hardware never rejects on EXT1, so a held pin sleeps and wakes; deferred (ADR-003 point 1) |
| `try_sleep` post-rejection postcondition           | bench          | bench          | compile                                | bench      | bench         | —         | —       | Non-wake pads and holds benched on S3 only (R4-S3); esp-hal S3 leaves `WIFI_FORCE_ISO` cleared (excluded power-domain bit)                            |
| Pad-autohold hang guard in `try_sleep`             | n/a (pre-read) | bench (R3)     | compile                                | n/a        | bench (R3)    | —         | —       | HW: a rejection after digital-pad isolation hangs; C2/C3 non-pin rejects with the hold on stay unguarded                                              |
| `WakeCauseSource`                                  | bench          | bench          | bench (Feather V2, timer)              | bench      | bench         | —         | —       | A rejection clears the EXT1 pin mask on S3, both tiers: capture before the first `try_sleep`                                                          |
| `ResetReasonSource`                                | bench          | bench          | compile                                | bench      | build         | —         | —       | UD on hal ESP32/C6                                                                                                                                    |
| `PowerRailGate` (`GpioRailGate`)                   | compile        | build          | compile                                | compile    | build         | compile   | compile |                                                                                                                                                       |
| `BatteryMonitor` (`EspAdcBatteryMonitor`)          | compile        | bench (Heltec) | bench (Feather V2)                     | —          | —             | —         | —       | UD on the whole hal tier (this feature doc)                                                                                                           |
| `ChargingMonitor` (`EspChargingMonitor`)           | compile        | compile        | compile                                | —          | —             | —         | —       | UD on hal. HW on Feather V2 (no STAT/VBUS GPIO) and Heltec V3 (TP4054 status LED-only); no battery-present signal on either board                     |
| RTC retention (`RetainedCell` / `RtcRetainedCell`) | bench (R1)     | build          | compile                                | bench (R1) | bench (knob)  | compile   | compile | IDF needs the opt-in `critical-section-impl` provider (ADR-003 point 5)                                                                               |
| RTC clock (`EspRtcClock` / `EspHalRtcClock`)       | build          | bench (R2-S3)  | compile                                | build      | bench (R2-S3) | —         | —       | UD on hal ESP32/C6                                                                                                                                    |

Deferred by decision (2026-11-04): all ESP32 hardware work (the EXT1 pre-read, the guard bench), `async` on both tiers, and a `WIFI_FORCE_ISO` restore on esp-hal.
The conformance modules (`src/conformance.rs`, ADR-003 point 4) keep the built cells' signatures in sync; the behaviour columns are bench results, not compile results.

## Decisions
|                                      Decision | Reason                                                               | Rejected Alternative                 |
|----------------------------------------------:|:---------------------------------------------------------------------|:-------------------------------------|
| Stage: ADC + charging in v1, sleep/wake in v2 | Sleep contract is IDF-shaped and needs an ADR                        | Full parity in one feature           |
|    Sleep module ships before ADC (2026-10-12) | Spookhouse C3 deadline; see esp-hal-sleep-tier-v1                    | ADC + charging first                 |
|         New crate `rustyfarian-esp-hal-power` | Matches ws2812/network/peripherals tier layout                       | Feature flag on the IDF crate        |
| `stoker` becomes `#![no_std]` unconditionally | One API shape for both tiers                                         | `std` feature keeping `anyhow`       |
|   Replace `anyhow` with a `stoker` error enum | `anyhow` blocks no_std; IDF crate maps at its edge                   | Associated `type Error` per trait    |
| `SleepManager` gets `type Error` (2026-10-12) | IDF impl must surface platform errors; enum stays the portable error | Fixed `SleepError` with IDF variants |
|    Chips: C3, C6, ESP32, S3 via chip features | Same four chip features as the sibling hal crates                    | ESP32 + S3 only                      |
|          Blocking API plus an `async` feature | Mirrors ws2812/network (esp-rtos + embassy)                          | Blocking only                        |

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
- [x] How should `stoker`'s `build.rs` derive the wake-pin mask for `*-none-elf` triples — caller-supplied mask with chip constants, decided in `archive/c3-gpio-deep-sleep-wake-v1.md`.

## State
- [x] Design approved
- [x] `stoker` no_std + error enum (IDF crate migrated)
- [x] `rustyfarian-esp-hal-power` scaffold, `just` hal recipes, `target/hal`
- [ ] ADC battery monitor + charging monitor (blocking)
- [ ] `async` feature
- [ ] Examples build for all four chips
- [ ] Hardware-validated on Heltec V3 and Feather V2
- [ ] Tests passing (`just verify`)
- [ ] Documentation updated (READMEs, key-insights, CHANGELOG)

## Session Log
- 2026-09-26 — Feature doc created via /feature dialog
- 2026-10-12 — Updated: sleep module pulled forward to its own doc; build.rs mask question answered by the C3 wake feature
- 2026-10-12 — `stoker` is `#![no_std]`, `anyhow` replaced by `SleepError`; `SleepManager` got `type Error` (amends the error-enum decision)
- 2026-10-12 — Scaffold landed: crate, chip features, `default-members`, `check-hal`/`clippy-hal`/`clean-hal` + release recipes into `target/hal`; sleep module next (esp-hal-sleep-tier-v1)
- 2026-10-12 — ADR-001 (HAL-neutral sleep contract) written and linked; the hal crate carries the compile-only pin-borrow spike until the sleep module replaces it
