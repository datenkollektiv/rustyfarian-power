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
Both tiers target C3, C6, ESP32 and S3 (IDF C6 added 2026-11-04).

| Capability (`stoker` trait or type)                                   | IDF C3         | IDF C6            | IDF S3         | IDF ESP32                              | hal C3     | hal C6            | hal S3        | hal ESP32 | Class and note                                                                                                                                                                                             |
|:----------------------------------------------------------------------|:---------------|:------------------|:---------------|:---------------------------------------|:-----------|:------------------|:--------------|:----------|:-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `SleepManager::sleep` / `try_sleep`, GPIO + timer                     | bench (R2)     | build             | bench (R2-R4)  | timer bench (Feather V2), GPIO compile | bench (R2) | build             | bench (R2-R4) | build     | BD on IDF ESP32 closed in software 2026-11-04 (guard built); both tiers reject multi-pin `AnyLow` on ESP32 (kind `Unsupported`); benches in final pass                                                     |
| `try_sleep` post-rejection postcondition                              | bench          | build             | bench          | compile                                | bench      | build             | bench         | build     | Non-wake pads and holds benched on S3 only (R4-S3); esp-hal S3 leaves `WIFI_FORCE_ISO` cleared (excluded power-domain bit)                                                                                 |
| Pad-autohold hang guard in `try_sleep`                                | n/a (pre-read) | n/a (guard Never) | bench (R3)     | build                                  | build      | n/a (guard Never) | bench (R3)    | build     | HW: a rejection after digital-pad isolation hangs; C6 guard `Never` pending the held-pin bench; ESP32 guard `Always` built                                                                                 |
| `WakeCauseSource`                                                     | bench          | build             | bench          | bench (Feather V2, timer)              | bench      | build             | bench         | build     | A rejection clears the EXT1 pin mask on S3, both tiers: capture before the first `try_sleep`                                                                                                               |
| `ResetReasonSource`                                                   | bench          | build             | bench          | compile                                | bench      | build             | build         | build     | Hal C6 and ESP32 reset reasons built, not yet benched                                                                                                                                                      |
| `PowerRailGate` (`GpioRailGate`)                                      | compile        | compile           | build          | compile                                | compile    | compile           | build         | compile   |                                                                                                                                                                                                            |
| `BatteryMonitor` (`EspAdcBatteryMonitor` / `EspHalAdcBatteryMonitor`) | compile        | compile           | bench (Heltec) | bench (Feather V2)                     | compile    | compile           | build         | bench     | Hal monitor added 2026-11-04 (ADC1 only, calibrated); hal C3/C6 have no example, so compile-only; hal ESP32 matches IDF within 2 mV on the Feather V2 (2026-11-08)                                                                                                           |
| `ChargingMonitor` (`EspChargingMonitor`)                              | compile        | compile           | compile        | compile                                | compile    | compile           | compile       | compile   | Shared `GpioChargingMonitor` (ADR-004 rung 1), hal tier is `EspHalChargingMonitor`. HW on Feather V2 (no STAT/VBUS GPIO) and Heltec V3 (TP4054 status LED-only); no battery-present signal on either board |
| RTC retention (`RetainedCell` / `RtcRetainedCell`)                    | bench (R1)     | build             | build          | compile                                | bench (R1) | compile           | bench (knob)  | compile   | IDF needs the opt-in `critical-section-impl` provider (ADR-003 point 5); IDF C6 example links, `.rtc_noinit` placement confirmed at build time                                                             |
| RTC clock (`EspRtcClock` / `EspHalRtcClock`)                          | build          | compile           | bench (R2-S3)  | compile                                | build      | compile           | bench (R2-S3) | compile   | Hal ESP32/C6 clock added 2026-11-04; compile-only, no example links it yet                                                                                                                                 |

Deferred by decision (2026-11-04): the ESP32 guard bench (the EXT1 pre-read is built), `async` on both tiers, and a `WIFI_FORCE_ISO` restore on esp-hal.
The conformance modules (`src/conformance.rs`, ADR-003 point 4) keep the built cells' signatures in sync; the behaviour columns are bench results, not compile results.

## Hardware pass (final, before release; not a gate)

- [x] ESP32 Feather V2: hal ADC reads within a few tens of mV of `idf_esp32_battery` (confirms `AdcCalLine`). Done 2026-11-08: hal 4132 mV vs IDF 4134 mV, two runs each (`just bench esp32 battery --unattended`).
- [ ] Heltec V3 (S3): hal ADC on GPIO1 matches `idf_esp32s3_battery` (about 80 % with USB and no cell).
- [ ] ESP32 WROOM or Feather V2: timer and single-pin EXT1 wake report `Ext1` with bit 25 set on hal and IDF.
- [ ] ESP32: held-pin `try_sleep` returns `Rejected` on both tiers; multi-pin `AnyLow` returns `AnyLowMultiplePins` (kind `Unsupported`) on both tiers; single-pin `AnyLow` wakes.
- [ ] ESP32: hal UART2 drain does not report `UartBusy` or hang while the console prints.
- [ ] C6-DevKitC-1: timer wake gives `reset=DeepSleep cause=Timer`; EXT1 on GPIO2 gives `Ext1` mask `0x4` on both tiers.
- [ ] C6-DevKitC-1: held GPIO2 at its wake level; `Rejected` keeps `Never`, an immediate wake switches the policy to `Always`.
- [ ] C6-DevKitC-1: hal UART0/UART1 drain does not time out (`fsm_status` path is unproven on hardware).
- [ ] C6-DevKitC-1: `EspHalRtcClock` and `EspRtcClock` stay monotonic across deep sleep.
- [ ] C6-DevKitC-1: retention cycle keeps the session id across deep sleep, software reset, panic and watchdog.
- [ ] C3-DevKitM-1: hal ADC against a known divider; regression R1-R4.
- [ ] Heltec V3 (S3): hal ADC on GPIO1 against IDF; regression R2-R4.
- [ ] C6-DevKitC-1: for the held-pin case judge the next boot too: reset reason and wake cause must not show an immediate EXT1 wake.
- [ ] Every chip, both tiers: three `try_sleep` retries after a software and after a hardware `Rejected` each behave as fresh attempts (no stale wake source, no held pad).
- [ ] C3 IDF: after a software `Rejected` the wake pin's deep-sleep enable is cleared (pin reads as a plain input, no immediate wake on a later sleep).
- [ ] Every chip, hal tier: a shortest-timer sleep under continuous UART output and GPIO interrupts neither misses its deadline nor reports `UartBusy`.
- [ ] Every chip, both tiers: the retention cycle keeps its session id across deep sleep and the reset kinds each tier supports.
- [ ] Every chip, both tiers: the reported wake mask matches the physical pin and polarity for `AnyHigh` and `AnyLow`.

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
- [ ] Which battery-equipped C3/C6 boards get `BatteryConfig` presets, or are they compile-only?
- [x] Which esp-hal calibration scheme per chip matches the IDF tier's mV readings within tolerance? `AdcCalLine` on ESP32, `AdcCalCurve` on C3, C6 and S3; the Feather V2 bench confirms it.
- [x] How should `stoker`'s `build.rs` derive the wake-pin mask for `*-none-elf` triples — caller-supplied mask with chip constants, decided in `archive/c3-gpio-deep-sleep-wake-v1.md`.

## State
- [x] Design approved
- [x] `stoker` no_std + error enum (IDF crate migrated)
- [x] `rustyfarian-esp-hal-power` scaffold, `just` hal recipes, `target/hal`
- [x] Charging monitor (shared `GpioChargingMonitor`, both tiers)
- [x] ADC battery monitor (blocking) on all four hal chips, `EspHalAdcBatteryMonitor`
- [ ] `async` feature
- [x] Examples build for all four chips (link builds per `ci-hal`, `ci-c6`, `ci-c3`, `ci-s3`, `ci-esp32`)
- Hardware validation: a single final pass follows the tier-parity changes, before the release; it is not a gate for this feature (Heltec V3 and Feather V2)
- [x] Tests passing (`just verify`)
- [ ] Documentation updated (READMEs, key-insights, CHANGELOG)

## Session Log
- 2026-09-26 — Feature doc created via /feature dialog
- 2026-10-12 — Updated: sleep module pulled forward to its own doc; build.rs mask question answered by the C3 wake feature
- 2026-10-12 — `stoker` is `#![no_std]`, `anyhow` replaced by `SleepError`; `SleepManager` got `type Error` (amends the error-enum decision)
- 2026-10-12 — Scaffold landed: crate, chip features, `default-members`, `check-hal`/`clippy-hal`/`clean-hal` + release recipes into `target/hal`; sleep module next (esp-hal-sleep-tier-v1)
- 2026-10-12 — ADR-001 (HAL-neutral sleep contract) written and linked; the hal crate carries the compile-only pin-borrow spike until the sleep module replaces it
- 2026-11-04 — Charging monitor shared as `stoker::charging::GpioChargingMonitor`; hardware gates removed from all feature docs, one final hardware pass precedes the release
- 2026-11-04 — Tier-parity items 1-5 landed: IDF C6, hal ADC (all chips), hal wake and reset (ESP32/C6), hal sleep and RTC clock (ESP32/C6); table gains IDF C6 and filled hal ESP32/C6 cells
- 2026-11-08 — Feather V2 (CH9102F, chip v3.1) benched with `rustyfarian-bench` v0.3.0 on USB, no cell: the charger rail reads hal 4132 mV (94 %) and IDF 4134 mV (94 %) in both of two runs, so `AdcCalLine` matches the IDF calibration within 2 mV; accuracy against a meter on a real cell stays open (attended step)
