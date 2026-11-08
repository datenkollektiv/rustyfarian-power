---
gate: none
desk-work: available
---

# Feature: esp-hal RTC Clock v1

`EspHalRtcClock::now_us() -> u64` in `rustyfarian-esp-hal-power`, the esp-hal twin of the IDF `EspRtcClock`, so a slept time read across two boots is off by a share of the interval, not of the uptime.
Requested by an upstream consumer on the esp-hal tier (filed 2026-10-24); not blocking, the consumer has an awake workaround.
Measured on the DevKitM-1: `Rtc::time_since_power_up()` differences for a 60 s sleep drift to −8.5 %..+6.1 % after an hour, about 0.14 % of uptime.
The cause: every boot recalibrates the slow clock and the whole 48-bit count is converted with this boot's period.
ESP-IDF avoids this by keeping accumulated microseconds and the last tick count in RTC memory and converting only the ticks since the last read (`esp_clk.c`).
The consumer measured +0.7 % steady on the IDF tier.

## Decisions
|                                                                       Decision | Reason                                                                        | Rejected Alternative                                                       |
|-------------------------------------------------------------------------------:|:------------------------------------------------------------------------------|:---------------------------------------------------------------------------|
|         IDF-style accumulation of `(last_ticks, accumulated_us)` in RTC memory | Error proportional to the interval, same bound as IDF; monotonic across boots | Raw ticks with one fixed period: no state, but drifts with temperature     |
|           The tier reads `RTC_TIMER` and the calibration store register itself | esp-hal 1.2.2 keeps `time_since_boot_raw` and `rtc_ticks_to_us` `pub(crate)`  | Upstream esp-hal change; the consumer pins 1.2.2                           |
| Period from esp-hal's boot calibration in `LP_AON STORE1` (19 fractional bits) | Same source esp-hal and IDF use; delta ticks only, so uptime drops out        | Longer per-boot calibration: shrinks scatter, keeps uptime proportionality |
|                 Shape `EspHalRtcClock::now_us() -> u64`, twin of `EspRtcClock` | One consumer call site for both tiers                                         | `WakeCause::Timer { slept_ms }`, rejected in ADR-001                       |
|                                          esp-hal clock only, no `stoker` trait | One consumer; ADR-001 reserves an additive trait for a second one             | `RtcClock` trait in `stoker` now                                           |
|                         CRC-32 envelope on `Accum` (`Retainable`, 20 bytes LE) | Rejects garbage after flashing and torn writes; reuses `stoker::retain`       | Two plain persistent `u64`, as IDF does                                    |
|                 `frac_q19` keeps the sub-µs remainder (deviates from the pair) | Frequent polling does not lose about 1 µs per call                            | Whole-µs pair `(last_ticks, accumulated_us)` only                          |
|            The clock reads the reset reason itself on the first call of a boot | ROM latches the cause, so a late first call works; `Retained::load` needs it  | Consumer passes the reset reason in                                        |

## Constraints
- esp-hal stays pinned at 1.2.2; no upstream change is required.
- The tier drives the RTC registers directly, as `sleep.rs` already does for GPIO; the read runs in a critical section.
- The accumulator restarts at zero on `PowerOn` and `Brownout` and survives `Software`, `Panic` and `Watchdog` resets and deep sleep, as `EspRtcClock` does.
- C3 is the reference chip and the bench target; the S3 must at least build, as for the sleep module.
- `no_std`, no allocation, no new `stoker` dependency.
- Module docs state what survives which reset, as the IDF module does.

## Open Questions
- [x] Where the accumulator lives: a private `RtcRetainedCell<20>` static, so `now_us()` takes no arguments.
- [x] How the clock learns the reset reason: it reads the latched ROM cause on the first call of each boot through `EspHalResetReasonSource`.
- [x] CRC envelope: yes, `Accum` implements `stoker::retain::Retainable` (see Decisions).
- [x] Release: an additive `[Unreleased]` Added entry folded into the pending 0.3.0; ADR-001 slept-time paragraph amended.
- [x] Mixing with `Rtc::time_since_power_up()`: documented, not forbidden; safe on C3, and on S3 call it from one core only.
- [x] Compile-only spike against esp-hal 1.2.2 answered the first three; it compiled for C3 and S3.

## Acceptance
- Ten consecutive 60 s timer sleeps after at least one hour since power-on: every difference within ±1 % of the wall clock.
- A 0.5 s sleep after one hour of uptime reads 0.4–0.7 s, never a negative difference.
- Readings survive a `Software` reset and restart only on `PowerOn`/`Brownout`, stated in the module docs.

## State
- [x] Design approved
- [x] Spike: register read, calibration read and retained accumulator compile for C3 and S3
- [x] Core implementation: `EspHalRtcClock` module, re-export, module docs
- [x] Example prints `rtc_us` at each boot (extend `hal_c3_rtc_retention` or a twin)
- [x] ESP32 path (2026-11-04): 20 `time_update` pulses, wait for `time_valid`, read `time1.time_hi` and `time0.time_lo`; `STORE1` on `LPWR`; arithmetic stays in `stoker::rtc_clock`
- [x] C6 path (2026-11-04): latch via `update.main_timer_update`, ticks from `main_buf0_high/low`, calibration from `LP_AON` `store1`; no delay needed on C6
- Hardware checks for the final pass: ESP32 clock monotonic across deep sleep (done 2026-11-08, WROOM, after the fast-memory fix) and within drift of `EspRtcClock` (open); C6 no backwards steps and elapsed time matches sleep time
- Hardware validation: a single final pass follows the tier-parity changes, before the release; it is not a gate for this feature (Acceptance bench on the hal C3 retention rig)
- [x] ADR-001 slept-time paragraph amended; CHANGELOG `[Unreleased]` Added entry
- [x] Tests passing (`just verify`, `just check-hal`, `just build-example-hal-c3` for the example)

## Session Log
- 2026-10-24 — Feature doc created via /feature dialog from the upstream request; approach fixed to IDF-style accumulation, storage and release questions left open
- 2026-10-28 — Spike compiled for C3 and S3 against esp-hal 1.2.2; `stoker::rtc_clock` core and host tests landed; Open Questions resolved per the approved plan; bench and verification pending
- 2026-11-04 — Hardware gate removed; benches move to the final pre-release hardware pass
- 2026-11-08 — Bench (Feather V2, `hal_esp32_deep_sleep_gpio` now prints `RTC rtc_us`): the hal ESP32 clock restarts at 0 on every `DeepSleep` boot (three boots: 0 → 577, 0 → 622, 0 → 611 µs), so it is NOT monotonic across deep sleep; open defect. First hypothesis (fast-memory power-down via esp-hal's `rtc_fastmem_pd_en`) weakened by review: the ESP32 `apply()` never writes `fastmem_pd_en`; see the next entry. `RtcRetainedCell` on the hal ESP32 would be affected the same way.
- 2026-11-08 — Confirmed with the new `hal_esp32_rtc_retention` (Feather V2, `just bench esp32 retention --unattended`): after a 5 s timer wake the record reads `reset=DeepSleep wake=Timer load=err:Magic boots=1` with a new session id, so `.rtc_fast.persistent` does not survive esp-hal deep sleep on the ESP32 (the C3 twin passes); `RtcRetainedCell` and `EspHalRtcClock` are both affected on the hal ESP32. Cause open: esp-hal 1.2.2 never writes `fastmem_pd_en` on the ESP32 (`sleep/esp32.rs:349`, review 2026-11-08), so a power-down is not established; experiment under way: force RTC fast memory on (`fastmem_force_pu`/`noiso`) in the ESP32 sleep path and rerun on a WROOM. Next suspects: the ROM or the IDF v5.3.3 second-stage bootloader touching 0x3FF80000, or a reset reason outside esp-hal's enum (which zeroes the section).
- 2026-11-08 — Fixed and bench-confirmed on an ESP32-WROOM-32 (`just bench esp32 retention --unattended`): before, with esp-hal's `RtcSleepConfig::deep()`, `reset=DeepSleep wake=Timer load=err:Magic boots=1`; after, with `deep_sleep_config()` forcing RTC fast memory on (`fastmem_force_pu`/`force_lpu`/`force_noiso`) on the ESP32, `load=ok`, `boots` 1 → 2 → 3, one session, and `EspHalRtcClock` continues (5.150 s and 5.158 s per 5 s cycle, as on the C3). One run each; deep-sleep current impact not yet measured (PPK2).
- 2026-11-08 — ESP32-WROOM-32, 60 s timer (`just bench esp32 sleep --unattended`, after the fast-memory fix): `EspHalRtcClock` continues across three deep sleeps; sleep-entry to next-boot readings 59.962 s, 60.154 s, 60.122 s (timer plus ~150 ms boot). The first cycle after power-on reads ~190 ms short of the others (calibration settling after the seed, unconfirmed); all within ±0.4 %. Drift against `EspRtcClock` still open.
- 2026-11-08 — Feather V2, same 60 s run: 59.963 s, 60.154 s, 60.154 s. The short first cycle after power-on (~190 ms) reproduces on a second board, so it is systematic, not noise; cause still unconfirmed (calibration right after the seed, or the first timer arm), and it does not affect monotonicity.
- 2026-11-08 — IDF comparison (`idf_esp32_deep_sleep_gpio`, Feather V2, `EspRtcClock`): 60.428 s for every cycle including the first, against hal 59.963/60.154/60.154 s; with IDF booting to `app_main` in ~419 ms and hal in ~150 ms, both imply ~60.0 s of sleep, so the hal clock agrees with `EspRtcClock` within ~0.1 % (estimated boot times, separate runs, not a same-boot comparison). The short first cycle after power-on is hal-only (the clock seed or esp-hal's first timer arm), still unexplained.
