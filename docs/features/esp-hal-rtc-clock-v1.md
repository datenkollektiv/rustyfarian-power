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
|                                                                       Decision | Reason                                                                            | Rejected Alternative                                                       |
|-------------------------------------------------------------------------------:|:----------------------------------------------------------------------------------|:---------------------------------------------------------------------------|
|         IDF-style accumulation of `(last_ticks, accumulated_us)` in RTC memory | Error proportional to the interval, same bound as IDF; monotonic across boots     | Raw ticks with one fixed period: no state, but drifts with temperature     |
|           The tier reads `RTC_TIMER` and the calibration store register itself | esp-hal 1.2.2 keeps `time_since_boot_raw` and `rtc_ticks_to_us` `pub(crate)`      | Upstream esp-hal change; the consumer pins 1.2.2                           |
| Period from esp-hal's boot calibration in `LP_AON STORE1` (19 fractional bits) | Same source esp-hal and IDF use; applied to delta ticks only, so uptime drops out | Longer per-boot calibration: shrinks scatter, keeps uptime proportionality |
|                 Shape `EspHalRtcClock::now_us() -> u64`, twin of `EspRtcClock` | One consumer call site for both tiers                                             | `WakeCause::Timer { slept_ms }`, rejected in ADR-001                       |
|                                          esp-hal clock only, no `stoker` trait | One consumer; ADR-001 reserves an additive trait for a second one                 | `RtcClock` trait in `stoker` now                                           |

## Constraints
- esp-hal stays pinned at 1.2.2; no upstream change is required.
- The tier drives the RTC registers directly, as `sleep.rs` already does for GPIO; the read runs in a critical section.
- The accumulator restarts at zero on `PowerOn` and `Brownout` and survives `Software`, `Panic` and `Watchdog` resets and deep sleep, as `EspRtcClock` does.
- C3 is the reference chip and the bench target; the S3 must at least build, as for the sleep module.
- `no_std`, no allocation, no new `stoker` dependency.
- Module docs state what survives which reset, as the IDF module does.

## Open Questions
- [ ] Where the accumulator lives: a private static inside the crate so `now_us()` stays argument-free, or a cell the consumer passes in.
- [ ] How the clock learns the reset reason to zero itself on `PowerOn`/`Brownout`: read the latched ROM reset cause on the first call of a boot, or take it from the consumer.
- [ ] Whether the accumulator uses the CRC envelope (`RtcRetainedCell` with a `Retainable` pair) or two plain persistent `u64` as IDF does.
- [ ] Release: 0.3.0, or additive in 0.3.x with the ADR-001 slept-time paragraph amended with the uptime-drift finding.
- [ ] Whether a consumer's own `Rtc::time_since_power_up()` call races the tier's snapshot read (esp-hal takes a private lock); document or forbid mixing them.
- [ ] A compile-only spike against esp-hal 1.2.2 answers the first three before the module is written.

## Acceptance
- Ten consecutive 60 s timer sleeps after at least one hour since power-on: every difference within ±1 % of the wall clock.
- A 0.5 s sleep after one hour of uptime reads 0.4–0.7 s, never a negative difference.
- Readings survive a `Software` reset and restart only on `PowerOn`/`Brownout`, stated in the module docs.

## State
- [x] Design approved
- [ ] Spike: register read, calibration read and retained accumulator compile for C3 and S3
- [ ] Core implementation: `EspHalRtcClock` module, re-export, module docs
- [ ] Example prints `rtc_us` at each boot (extend `hal_c3_rtc_retention` or a twin)
- [ ] Bench on the hal C3 retention rig per Acceptance, results in `docs/bench/`
- [ ] ADR-001 slept-time paragraph amended; CHANGELOG `[Unreleased]` Added entry
- [ ] Tests passing (`just verify`, `just check-hal`, `just build-example-hal-c3` for the example)

## Session Log
- 2026-10-24 — Feature doc created via /feature dialog from the upstream request; approach fixed to IDF-style accumulation, storage and release questions left open
