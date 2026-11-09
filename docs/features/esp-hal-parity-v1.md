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
| `SleepManager::sleep` / `try_sleep`, GPIO + timer                     | bench (R2)     | build             | bench (R2-R4)  | bench (Feather V2)                     | bench (R2) | build             | bench (R2-R4) | bench     | BD on IDF ESP32 closed in software 2026-11-04 (guard built); both tiers reject multi-pin `AnyLow` on ESP32 (kind `Unsupported`); benches in final pass; ESP32 timer, EXT1 bit 25 and held-pin `Rejected` benched on both tiers 2026-11-08 (Feather V2); multi-pin `AnyLow` refusal and single-pin `AnyLow` wake done on both tiers 2026-11-09 (`esp32 anylow`/`anylow-multi`; single-pin mask needs the shared `stoker::ext1_fired_mask` armed-record fallback) |
| `try_sleep` post-rejection postcondition                              | bench          | build             | bench          | compile                                | bench      | build             | bench         | build     | Non-wake pads and holds benched on S3 only (R4-S3); esp-hal S3 leaves `WIFI_FORCE_ISO` cleared (excluded power-domain bit)                                                                                 |
| Pad-autohold hang guard in `try_sleep`                                | n/a (pre-read) | n/a (guard Never) | bench (R3)     | build                                  | build      | n/a (guard Never) | bench (R3)    | build     | HW: a rejection after digital-pad isolation hangs; C6 guard `Never` pending the held-pin bench; ESP32 guard `Always` built                                                                                 |
| `WakeCauseSource`                                                     | bench          | build             | bench          | bench (Feather V2)                     | bench      | build             | bench         | bench     | A rejection clears the EXT1 pin mask on S3, both tiers: capture before the first `try_sleep`                                                                                                               |
| `ResetReasonSource`                                                   | bench          | build             | bench          | compile                                | bench      | build             | build         | build     | Hal C6 and ESP32 reset reasons built, not yet benched                                                                                                                                                      |
| `PowerRailGate` (`GpioRailGate`)                                      | compile        | compile           | build          | compile                                | compile    | compile           | build         | compile   |                                                                                                                                                                                                            |
| `BatteryMonitor` (`EspAdcBatteryMonitor` / `EspHalAdcBatteryMonitor`) | compile        | compile           | bench (Heltec) | bench (Feather V2)                     | compile    | compile           | bench (Heltec) | bench     | Hal monitor added 2026-11-04 (ADC1 only, calibrated); hal C3/C6 have no example, so compile-only; hal ESP32 matches IDF within 2 mV on the Feather V2 (2026-11-08); hal S3 matches IDF within 4 mV raw on the Heltec V3.1, meter-confirmed at 4.18 V (2026-11-09, `battery-calib`)                                                                                                           |
| `ChargingMonitor` (`EspChargingMonitor`)                              | compile        | compile           | compile        | compile                                | compile    | compile           | compile       | compile   | Shared `GpioChargingMonitor` (ADR-004 rung 1), hal tier is `EspHalChargingMonitor`. HW on Feather V2 (no STAT/VBUS GPIO) and Heltec V3 (TP4054 status LED-only); no battery-present signal on either board |
| RTC retention (`RetainedCell` / `RtcRetainedCell`)                    | bench (R1)     | build             | build          | compile                                | bench (R1) | compile           | bench (knob)  | bench     | IDF needs the opt-in `critical-section-impl` provider (ADR-003 point 5); IDF C6 example links, `.rtc_noinit` placement confirmed at build time; hal ESP32 bench 2026-11-08 (WROOM, needs RTC fast memory forced on in deep sleep) |
| RTC clock (`EspRtcClock` / `EspHalRtcClock`)                          | build          | compile           | bench (R2-S3)  | compile                                | build      | compile           | bench (R2-S3) | bench     | Hal ESP32/C6 clock added 2026-11-04; compile-only, no example links it yet; hal ESP32 continues across deep sleep, bench 2026-11-08 (WROOM) |

Deferred by decision (2026-11-04): the ESP32 guard bench (the EXT1 pre-read is built), `async` on both tiers, and a `WIFI_FORCE_ISO` restore on esp-hal.
The conformance modules (`src/conformance.rs`, ADR-003 point 4) keep the built cells' signatures in sync; the behaviour columns are bench results, not compile results.

## Hardware pass (final, before release; not a gate)

- [x] ESP32 Feather V2: hal ADC reads within a few tens of mV of `idf_esp32_battery` (confirms `AdcCalLine`). Done 2026-11-08: hal 4132 mV vs IDF 4134 mV, two runs each (`just bench esp32 battery --unattended`).
- [x] Heltec V3 (S3): hal ADC on GPIO1 matches `idf_esp32s3_battery`. Done 2026-11-09 with a pack fitted: meter 4.18 V, hal 4191 mV / IDF 4176 mV (raw 830 vs 827), both tiers on eFuse curve fitting, `divider_ratio` 5.05 (`just bench heltec/battery-calib`, unattended and attended).
- [x] ESP32 WROOM or Feather V2: timer and single-pin EXT1 wake report `Ext1` with bit 25 set on hal and IDF. Done 2026-11-08 on both tiers (Feather V2, `just bench esp32 sleep` + `esp32 ext1-wake`).
- [x] ESP32: held-pin `try_sleep` returns `Rejected` on both tiers; multi-pin `AnyLow` returns `AnyLowMultiplePins` (kind `Unsupported`) on both tiers; single-pin `AnyLow` wakes. Held-pin `Rejected` done on both tiers 2026-11-08 (`esp32 ext1-wake`); multi-pin `AnyLow` done on both tiers 2026-11-09 (`esp32 anylow-multi`); single-pin `AnyLow` done on both tiers 2026-11-09 (`esp32 anylow`, 4/4 both tiers), the hal mask via the new armed-record fallback (session log below).
- [x] ESP32: hal UART2 drain does not report `UartBusy` or hang while the console prints. Done 2026-11-08: Feather V2, two timer wakes, no `UartBusy`, no hang (`just bench esp32 sleep --unattended`).
- [ ] C6-DevKitC-1: timer wake gives `reset=DeepSleep cause=Timer`; EXT1 on GPIO2 gives `Ext1` mask `0x4` on both tiers.
- [ ] C6-DevKitC-1: held GPIO2 at its wake level; `Rejected` keeps `Never`, an immediate wake switches the policy to `Always`.
- [ ] C6-DevKitC-1: hal UART0/UART1 drain does not time out (`fsm_status` path is unproven on hardware).
- [ ] C6-DevKitC-1: `EspHalRtcClock` and `EspRtcClock` stay monotonic across deep sleep.
- [ ] C6-DevKitC-1: retention cycle keeps the session id across deep sleep, software reset, panic and watchdog.
- [ ] C3-DevKitM-1: hal ADC against a known divider; regression R1-R4.
- [x] Heltec V3 (S3): hal ADC on GPIO1 against IDF. Done 2026-11-09 (see above).
- [ ] Heltec V3 (S3): regression R2-R4.
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
- 2026-11-08 — Feather V2, `hal_esp32_deep_sleep_gpio` with nothing wired (GPIO25 internal pull-down): power-on plus two `DEEPSLEEP_RESET` boots with `Wake cause: Timer`, GPIO25 LOW at every entry, no `UartBusy`, no hang; closes the ESP32 UART drain check; the log has no timestamps, so timer accuracy is not judged; EXT1 bit 25 and the IDF half stay open
- 2026-11-08 — Bench (Feather V2, `hal_esp32_deep_sleep_gpio` now prints `RTC rtc_us`): the hal ESP32 clock restarts at 0 on every `DeepSleep` boot (three boots: 0 → 577, 0 → 622, 0 → 611 µs), so it is NOT monotonic across deep sleep; open defect. First hypothesis (fast-memory power-down via esp-hal's `rtc_fastmem_pd_en`) weakened by review: the ESP32 `apply()` never writes `fastmem_pd_en`; see the next entry. `RtcRetainedCell` on the hal ESP32 would be affected the same way.
- 2026-11-08 — Confirmed with the new `hal_esp32_rtc_retention` (Feather V2, `just bench esp32 retention --unattended`): after a 5 s timer wake the record reads `reset=DeepSleep wake=Timer load=err:Magic boots=1` with a new session id, so `.rtc_fast.persistent` does not survive esp-hal deep sleep on the ESP32 (the C3 twin passes); `RtcRetainedCell` and `EspHalRtcClock` are both affected on the hal ESP32. Cause open: esp-hal 1.2.2 never writes `fastmem_pd_en` on the ESP32 (`sleep/esp32.rs:349`, review 2026-11-08), so a power-down is not established; experiment under way: force RTC fast memory on (`fastmem_force_pu`/`noiso`) in the ESP32 sleep path and rerun on a WROOM. Next suspects: the ROM or the IDF v5.3.3 second-stage bootloader touching 0x3FF80000, or a reset reason outside esp-hal's enum (which zeroes the section).
- 2026-11-08 — Fixed and bench-confirmed on an ESP32-WROOM-32 (`just bench esp32 retention --unattended`): before, with esp-hal's `RtcSleepConfig::deep()`, `reset=DeepSleep wake=Timer load=err:Magic boots=1`; after, with `deep_sleep_config()` forcing RTC fast memory on (`fastmem_force_pu`/`force_lpu`/`force_noiso`) on the ESP32, `load=ok`, `boots` 1 → 2 → 3, one session, and `EspHalRtcClock` continues (5.150 s and 5.158 s per 5 s cycle, as on the C3). One run each; deep-sleep current impact not yet measured (PPK2).
- 2026-11-08 — Feather V2, `just bench esp32 ext1-wake --attended` (jumper A1 → 3V3), esp-hal tier: single-pin EXT1 wake reports `Ext1` mask `0x2000000` (bit 25) on every touch; a held pin returns `kind=Rejected` and the example retries (twice, up to 320 retries). One hand-held touch read HIGH at boot, slept without a rejection and woke 230 ms later (contact bounce at the guard read, not a defect). The release step was inconclusive (rejections continued through the 15 s window), so the step now requires a timer wake after the release. Hal halves of the ESP32 EXT1 and held-pin items covered; the IDF halves and multi-pin `AnyLow` stay open, so the boxes stay unticked.
- 2026-11-08 — `esp32 ext1-wake` rerun with the stricter release step, 4/4: touch → `Ext1` mask `0x2000000`; hold → `kind=Rejected`, ≥ 140 retries, sleeps as soon as the wire is removed; release → next wake is `Timer` with GPIO25 LOW and no EXT1 wake. The timer wake came 67.3 s after the first rejected attempt (> 60 s + boot), so the successful attempt armed a fresh timer: partial evidence (ESP32 hal, software rejection) for the "retries behave as fresh attempts" item, not ticked.
- 2026-11-08 — New `idf_esp32_deep_sleep_gpio` (IDF twin) on the Feather V2: `esp32 sleep --unattended` 3/3 (three timer wakes, GPIO25 LOW, no EXT1 wake, so the pull-down holds in IDF EXT1 deep sleep; cycles 60.428 s ×3), `esp32 ext1-wake --attended` 4/4 twice (`Ext1` mask `0x2000000`; held pin `kind=Rejected` via `ESP_ERR_INVALID_STATE`, up to 520 retries; timer wake after the release, 64.9 s and 68.1 s after the first rejected attempt, so a fresh timer). Ticked the ESP32 EXT1 item; capability cells for ESP32 sleep and wake cause are bench on both tiers.
- 2026-11-09 — `esp32 anylow-multi` (bench variant, GPIO25 + GPIO26 `AnyLow`, Feather V2): both tiers refuse at every boot before arming with the identical message "AnyLow wake on several pins is not supported on this tier and chip; name a single pin or use AnyHigh", `kind=Unsupported`, and the timer-only fallback wakes on `Timer` each cycle. The hal timer-only cycles read 60.027 s then 60.186 s, so the short first cycle after power-on also appears without EXT1; IDF reads 60.446 s throughout.
- 2026-11-09 — `esp32 anylow` (variant `anylow`, GPIO25 pull-up, jumper to GND, Feather V2). Unattended both tiers 2/2: the pull-up holds, timer wakes only. Attended hal 4/4: `Ext1` mask `0x2000000`, held pin `Rejected`, timer wake after the release. Attended IDF failed step 2: every low-level EXT1 wake (3 of 3) reports `Ext1` with mask `0x0` (`GPIO25 fired: false`), while the wake itself and the held-pin `Rejected` work. Behavioural difference (ADR-003 point 2), IDF ESP32 `AnyLow` only: ESP-IDF's boot code clears the EXT1 pin selection (`cpu_start.c:752` → `esp_deep_sleep_wakeup_io_reset`) before `app_main` reads `esp_sleep_get_ext1_wakeup_status()`, and in low-level mode the status then reads empty (`AnyHigh` keeps bit 25); esp-hal records the status before it releases the pads. The mechanism inside the chip is inferred, not confirmed. Decision: work around, on both tiers (session log below).
- 2026-11-09 — Attended hal reruns the same morning (`anylow` 08:25, 08:27, 08:35): `fired pins: 0x0` on 5 of 5 touch wakes, including a 3 s solid hold with the pin Low at boot, on firmware identical to the passing 01:01 run (3 of 3 `0x2000000`). The hal `caused_wakeup()` read of the EXT1 status is therefore not reliable at every boot either. One shared flow fix: `armed_ext1` (`crates/rustyfarian-esp-hal-power/src/armed_ext1.rs`) keeps the armed mask and level in `.rtc_fast.persistent` (the only RTC region this tier keeps powered; `.rtc_slow` is lost under `RtcSleepConfig::deep()`), `BoundSleep::arm` invalidates at arm start and stores (`mask`, `Event::LowLevel`) after a non-empty pin arm, `disarm` invalidates, and `EspHalWakeCauseSource` resolves an empty status through the shared `stoker::ext1_fired_mask`. Gates green (`verify`, `clippy-hal`, `check-all`, `check-esp32`, bench-build); rerun passed 4/4 on both tiers (`anylow`, 08:57 hal / 09:00 idf) with `fired pins: 0x2000000, GPIO25 fired: true` on every touch wake, including boots where the register came up empty.
- 2026-11-09 — Fix for the IDF ESP32 `AnyLow` mask (user decision): `EspSleepManager` records the armed EXT1 mask and level in `.rtc_noinit` (invalidated at the start of every arm, written after a successful EXT1 arm), and `EspWakeCauseSource` resolves an empty low-level status to the single armed pin via the new pure `stoker::ext1_fired_mask` (3 host tests). Gates green (`verify`, `check-all`, `check-esp32`, `check-c3`, `clippy-all`, ESP32 example build); bench rerun of `esp32 anylow --attended` pending. `code-reviewer` follow-ups applied: the first `EspWakeCauseSource` call of a boot consumes the record (cached in RAM for repeat calls), a rolled-back arm invalidates it, two more host tests.
- 2026-11-09 — Heltec V3.1 battery: IDF tier switched to `Calibration::Curve`, `divider_ratio` 5.05 meter-confirmed (4.18 V), `battery-calib` passed in both modes; hal S3 `BatteryMonitor` cell is now bench
