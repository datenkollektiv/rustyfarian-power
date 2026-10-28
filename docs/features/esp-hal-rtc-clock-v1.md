---
gate: hardware
gate-note: Ten 60 s timer sleeps after one hour of uptime on the hal C3 retention rig, every slept time within ±1 %
desk-work: available
---

# Feature: esp-hal RTC 

`EspHalRtcClock::now_us() -> u64` in `rustyfarian-esp-hal-power`, the esp-hal twin of the IDF `EspRtcClock`, so a slept time read across two boots is off by a share of the interval, not of the uptime.
Requested by an upstream consumer on the esp-hal tier (filed 2026-10-24); not blocking, the consumer has an awake workaround.
Measured on the DevKitM-1: `Rtc::time_since_power_up()` differences for a 60 s sleep drift to −8.5 %..+6.1 % after an hour, about 0.14 % of uptime, because every boot recalibrates the slow clock and the whole 48-bit count is converted with this boot's period.
ESP-IDF avoids this by keeping accumulated microseconds and the last tick count in RTC memory and converting only the ticks since the last read (`esp_clk.c`); the consumer measured +0.7 % steady on the IDF tier.

## Decisions
|                                                                       Decision | Reason                                                                             | Rejected Alternative                                                       |
|-------------------------------------------------------------------------------:|:-----------------------------------------------------------------------------------|:---------------------------------------------------------------------------|
|         IDF-style accumulation of `(last_ticks, accumulated_us)` in RTC memory | Error proportional to the interval, same bound as IDF; monotonic across boots      | Raw ticks with one fixed period: no state, but drifts with temperature     |
|           The tier reads `RTC_TIMER` and the calibration store register itself | esp-hal 1.2.2 keeps `time_since_boot_raw` and `rtc_ticks_to_us` `pub(crate)`       | Upstream esp-hal change; the consumer pins 1.2.2                           |
| Period from esp-hal's boot calibration in `LP_AON STORE1` (19 fractional bits) | Same source esp-hal and IDF use; applied to delta ticks only, so uptime drops out  | Longer per-boot calibration: shrinks scatter, keeps uptime proportionality |
|                 Shape `EspHalRtcClock::now_us() -> u64`, twin of `EspRtcClock` | One consumer call site for both tiers                                              | `WakeCause::Timer { slept_ms }`, rejected in ADR-001                       |
|                                          esp-hal clock only, no `stoker` trait | One consumer; ADR-001 reserves an additive trait for a second one                  | `RtcClock` trait in `stoker` now                                           |
|                         CRC-32 envelope on `Accum` (`Retainable`, 20 bytes LE) | Rejects garbage after flashing and torn writes; reuses `stoker::retain`            | Two plain persistent `u64`, as IDF does                                    |
|                 `frac_q19` keeps the sub-µs remainder (deviates from the pair) | Frequent polling does not lose about 1 µs per call                                 | Whole-µs pair `(last_ticks, accumulated_us)` only                          |
|            The clock reads the reset reason itself on the first call of a boot | ROM latches the cause, so a late first call still works; `Retained::load` needs it | Consumer passes the reset reason in                                        |

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
- [ ] Bench on the hal C3 retention rig per Acceptance, results in `docs/bench/`
- [x] ADR-001 slept-time paragraph amended; CHANGELOG `[Unreleased]` Added entry
- [x] Tests passing (`just verify`, `just check-hal`, `just build-example-hal-c3` for the example)

## Session Log
- 2026-10-24 — Feature doc created via /feature dialog from the upstream request; approach fixed to IDF-style accumulation, storage and release questions left open
- 2026-10-28 — Spike compiled for C3 and S3 against esp-hal 1.2.2; `stoker::rtc_clock` core and host tests landed; Open Questions resolved per the approved plan; bench and verification pending
