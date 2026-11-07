---
gate: none
desk-work: available
---

# Feature: esp-hal Sleep Tier (C3-first) v1

Sleep and wake-cause support in `rustyfarian-esp-hal-power`, pulled forward from the parity doc's deferred v2 and scoped to the ESP32-C3 first.
Requested by rustbox-spookhouse for its esp-hal prop (`docs/features/upstream-sleep-wake-requests-v1.md` in that repo, filed 2026-10-12); its consumer requirements are in ADR-001.
Facts verified 2026-10-12 against esp-hal 1.2.2 in the cargo registry (`sleep_deep_with_rejection`, per-pin `apply_wakeup_config`, `caused_wakeup`).

## Decisions
|                                                                       Decision | Reason                                                                         | Rejected Alternative                           |
|-------------------------------------------------------------------------------:|:-------------------------------------------------------------------------------|:-----------------------------------------------|
|                                 Sleep module first in the hal crate, on the C3 | Consumer deadline; parity v1 re-ordered accordingly                            | Keep ADC + charging first; parallel modules    |
|                               ADR for a HAL-neutral sleep contract before code | The IDF-shaped `pin_mask` contract does not port (key-insights § esp-hal tier) | Driver-only port with ad-hoc API               |
|                                                  Pin ownership left to the ADR | Borrowing `&mut Input` at sleep time vs manager-owned pins needs a spike       | Decide here without one                        |
|                              Rejectable sleep over `sleep_deep_with_rejection` | Same `try_sleep` contract as the IDF tier (C3 wake feature)                    | `sleep_deep` only, panics on a held pin        |
| Manager pre-validates empty sources, zero/near-zero timers, `Instant` overflow | esp-hal panics on those; the trait returns errors                              | Let esp-hal panic                              |
|                         File the three esp-hal 1.2.2 findings as esp-rs issues | Upstream decides; links become external dependencies here                      | Record in key-insights only                    |
|                               Contract fixed in ADR-001 (2026-10-12, Proposed) | All contract points decided in one place [^adr]                                | Decide piecemeal across the three feature docs |
|            Wake pins borrowed as `&mut Input` per attempt via `bind_wake_pins` | Spike compiles on esp-hal 1.2.2; the firmware keeps polling the pin [^spike]   | Manager-owned pins                             |
|               `try_sleep` required on the trait; hal `type Error = SleepError` | One error type on the hal tier; rejection is `SleepError::Rejected`            | Default `try_sleep`; hal-local error enum      |
|  `EspHalWakeCauseSource::capture(&[&Input])` at boot, before first `try_sleep` | `caused_wakeup()` is per pin and a rejected sleep rewrites the record          | Read the cause lazily                          |

[^adr]: [ADR-001](../adr/001-hal-neutral-sleep-contract.md) decides trait shape, pin ownership, errors, pre-validation and the consumer follow-ups.
[^spike]: Spike check: `just check-hal-c3` exit 0; the firmware polling the pin is consumer point 1.

## Constraints
- `esp-hal =1.2.2` exact pin, MSRV 1.95 on the hal crate, separate `target/hal`, chip features C3/C6/ESP32/S3 as in parity v1.
- The esp-hal sleep API is `#[instability::unstable]`; every claim is pinned to 1.2.2.
- C3 LP pads are GPIO0-5; other pads return `WakeConfigError::NoLowPowerPath`.
- After a rejected sleep esp-hal does not restore every pad (ADR-001 § 4).
  - Wake pins are re-armed on every attempt.
  - They are released (`unlisten`, `clear_interrupt`, default `WakeupConfig`) when `try_sleep` returns `Rejected`.
  - Other digital pads are the firmware's to reconfigure.
- The whole request is validated before any pin or the deadline is touched; a later arming failure releases everything (ADR-001 § 4a).
- Arming, the sleep call and the post-rejection cleanup run inside one `critical_section::with`; a custom GPIO handler must not `unlisten` a bound pin (ADR-001 § 4b).
- Timer: request floor `MIN_TIMER_MS` and post-arm guard `POST_ARM_MARGIN_US` are provisional (1 ms, 200 µs) until the `short-timer-sweep` bench sets them (ADR-001 § 4c).
- Timer maximum `MAX_TIMER_MS = u32::MAX` ms (`TimerTooLong`): the RTC comparator is 48 bits and esp-hal drops the higher bits (ADR-001 § 4a).
- UART TX is drained, bounded by 200 ms (`UartBusy`), inside the critical section before the deadline is armed; esp-hal's own drain at sleep entry runs after both deadline checks (ADR-001 § 4c).
- Exclusivity: the binding is the complete set of low-power wake pins; an unbound listening pad on GPIO0-5 is `ForeignWakePin`, and firmware never calls `apply_wakeup_config` itself (ADR-001 § 4d).
- Wake pins need a pull against the wake level; esp-hal adds none and a floating pad wakes immediately.
- `WakeCause::Gpio(mask)` is assembled from `caused_wakeup()` on the registered pins; the mask shape comes from the C3 wake feature.
- Depends on the hal crate scaffold from parity v1 and on `stoker` being `no_std` with its error enum.
- Light sleep, async sleep, PM locks, ULP, and ESP32/C6 sleep on esp-hal are out of scope; the S3 joined via the knob request U4.

## Open Questions
- [x] Does the hal manager borrow `&mut Input` wake pins at sleep time, or own them?
  - Borrow per attempt; spike in `src/spike_pin_borrow.rs`, ADR-001 § 2.
  - The pin lifetime must be its own parameter, not the manager's `'d`.
- [x] Does esp-hal 1.2.2 deep sleep keep RTC fast memory on the C3 given `rtc_fastmem_pd_en` is never applied?
  - Yes, bench R1 on 2026-10-16: 23 consecutive wakes kept an `RtcRetainedCell` record (`docs/bench/c3-hal-rtc-retention-bench-v1.md`).
- [ ] Which esp-rs issue numbers track the three findings once filed?

## State
- [x] Design approved
- [x] ADR `docs/adr/001-hal-neutral-sleep-contract.md` written (Proposed) with the spike result recorded; linked from parity v1
- [x] ADR-001 review round 1 applied: validate-then-arm, interrupts masked across arm/sleep/cleanup, timer guarantee narrowed (spike attempt 3, `just check-hal-c3` exit 0)
- [x] ADR-001 review round 2 applied: timer maximum, UART drain before arming, foreign-wake-pin rule (spike attempt 4, `just check-hal-c3` exit 0)
- ADR-001 acceptance follows the final hardware pass, which sets the timer constants and runs the § 4b-4d cases
- [ ] Three esp-rs issues filed (GPIO light-sleep-only doc, `rtc_fastmem_pd_en` unused on C3, C3 metadata lists Touch/Ulp)
  - `src/rtc_cntl/sleep/mod.rs:4-7` says GPIO wake is light-sleep only, but LP pads wake from deep sleep.
  - `RtcSleepConfig::deep()` sets `rtc_fastmem_pd_en` for the C3 (`sleep/esp32c3.rs:295`), but `apply()` never reads it.
  - `_generated_esp32c3.rs:862-875` lists `Touch` and `Ulp` wake sources the C3 does not have.
- [x] `rustyfarian-esp-hal-power` sleep module: `SleepManager`, `WakeCauseSource`, `try_sleep` (C3 and S3, 2026-10-14)
  - S3 bench in [esp32s3-deep-sleep-gpio-wake-v1](archive/esp32s3-deep-sleep-gpio-wake-v1.md).
- [x] Example `hal_c3_deep_sleep_gpio`, twin of the IDF example, same wiring (built 2026-10-15 as part of plan item 1)
- [x] Sleep module on ESP32 and ESP32-C6, 2026-11-04
  - ESP32: `CHIP_POLICY = Esp32`, guard `Always`, multi-pin `AnyLow` rejected as `AnyLowMultiplePins`.
  - ESP32-C6: `Esp32C6`, guard `Never`, EXT1 mask `0xFF`.
  - The hal C6 `ReadHold` answers `HoldEffective(false)` because the chip has no `dig_iso`.
- [x] Examples `hal_esp32_deep_sleep_gpio` (GPIO25 PIR, EXT1) and `hal_c6_deep_sleep_gpio` (GPIO2 PIR, EXT1, timer fallback), 2026-11-04
- Hardware checks for the final pass:
  - ESP32 timer and single-pin EXT1 wake (`Ext1`, bit 25).
  - ESP32 held-pin `Rejected` and multi-pin `AnyLow` rejection.
  - ESP32 UART2 drain.
  - C6 timer wake (`reset=DeepSleep cause=Timer`).
  - C6 EXT1 wake (`Ext1`, mask `0x4`).
  - C6 held-pin test (`Never` or `Always`).
  - C6 UART0/UART1 drain (`fsm_status` path unproven).
  - Regression on C3 and S3.
- Hardware validation: a single final pass follows the tier-parity changes, before the release; it is not a gate for this feature.
  - C3 DevKitM-1: GPIO4 wake, timer fallback.
  - Bench case `wake-pin-held-at-entry` (expect `Rejected`).
  - Bench case `wake-cause-survives-rejection` (ADR-001 § 5).
  - Bench cases `held-level-custom-handler` and `rejection-retry-loop`.
  - Bench case `short-timer-sweep` with queued UART output and GPIO plus timer, measuring guard-to-sleep-entry.
  - Bench case `foreign-wake-pin` (ADR-001 § 4b-4d).
- [ ] Example documents GPIO4 = MTMS (JTAG) and the pull against the wake level
- rustbox-spookhouse measures deep-sleep current on both stacks from the same wiring, after the final hardware pass
- [ ] Tests passing (`just verify`, hal check recipe)
- [ ] CHANGELOG `[Unreleased]`: Added (hal sleep module)

## Session Log
- 2026-10-12 — Feature doc created via /feature dialog from the spookhouse request
- 2026-10-12 — ADR-001 written (Proposed):
  - Compile-only spike `spike_pin_borrow.rs` proves `&mut Input` borrowing at `try_sleep` on esp-hal 1.2.2 (`just check-hal-c3` exit 0 after decoupling the pin lifetime from the manager's).
  - Open questions and bench cases updated from the ADR.
- 2026-10-12 — Review round 1 on ADR-001:
  - § 4 rewritten (validation before mutation, critical section around arm/sleep/cleanup, post-arm timer guard, three new bench cases).
  - Spike attempt 3 compiles.
- 2026-10-12 — Review round 2 on ADR-001:
  - `TimerTooLong` (48-bit comparator), UART drain before arming (esp-hal drains after the deadline checks), exclusivity rule with `ForeignWakePin`.
  - Spike attempt 4 compiles.
- 2026-10-14 — Spike promoted to the exported `sleep` module with C3 and S3 arms (knob U4); the C3 hal bench is still open
- 2026-10-14 — Folded the spookhouse request and follow-up into this doc and ADR-001; review-queue copy retired
- 2026-10-15 — C3 hal tier bench prerequisites completed:
  - Examples `hal_c3_deep_sleep_gpio` and `hal_c3_sleep_bench` with 12 cases (W1-W3, H1-H8, R1).
  - `bench` feature in sleep module.
  - Recipes `build-example-hal-c3`, `run-hal-c3` and `flash-hal-c3`.
  - C3 toolchain in `.cargo/config.toml.dist`.
  - Bench plan updated with board info and case equipment table.
  - CHANGELOG [Unreleased] entries added.
- 2026-10-15 — Stimulus firmware and host script completed:
  - `examples/hal_esp32_bench_stimulus.rs` (ESP32-WROOM bare-metal, UART + Flex GPIO25/26).
  - `scripts/c3-bench.py` (Python parser and case state machines, `--self-test` on canned logs).
  - `just bench-c3` recipe.
  - ESP32 target in `.cargo/config.toml.dist`.
  - Bench plan rewritten with stimulus wiring, host procedure (flash-hal-esp32, bench-c3 commands, ZZ safe-state), protocol table, and MANUAL verification items.
  - MANUAL items: W1 50 ms edge-to-boot with LA, W2 ±5 % timer, H5 guard-to-entry with PPK2.
