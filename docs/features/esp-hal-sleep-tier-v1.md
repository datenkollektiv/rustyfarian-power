---
gate: none
desk-work: available
---

# Feature: esp-hal Sleep Tier (C3-first) v1

Sleep and wake-cause support in `rustyfarian-esp-hal-power`, pulled forward from the parity doc's deferred v2 and scoped to the ESP32-C3 first.
Requested by rustbox-spookhouse for its esp-hal prop; request copy in `review-queue/rustyfarian-power-esp-hal-sleep-tier-v1.md`.
Facts verified 2026-10-12 against esp-hal 1.2.2 in the cargo registry (`sleep_deep_with_rejection`, per-pin `apply_wakeup_config`, `caused_wakeup`).

## Decisions
|                                                                          Decision | Reason                                                                         | Rejected Alternative                        |
|----------------------------------------------------------------------------------:|:-------------------------------------------------------------------------------|:--------------------------------------------|
|                                    Sleep module first in the hal crate, on the C3 | Consumer deadline; parity v1 re-ordered accordingly                            | Keep ADC + charging first; parallel modules |
|                                  ADR for a HAL-neutral sleep contract before code | The IDF-shaped `pin_mask` contract does not port (key-insights § esp-hal tier) | Driver-only port with ad-hoc API            |
|                                                     Pin ownership left to the ADR | Borrowing `&mut Input` at sleep time vs manager-owned pins needs a spike       | Decide here without one                     |
|                                 Rejectable sleep over `sleep_deep_with_rejection` | Same `try_sleep` contract as the IDF tier (C3 wake feature)                    | `sleep_deep` only, panics on a held pin     |
| Manager pre-validates empty sources, zero or near-zero timers, `Instant` overflow | esp-hal panics on those; the trait returns errors                              | Let esp-hal panic                           |
|                            File the three esp-hal 1.2.2 findings as esp-rs issues | Upstream decides; links become external dependencies here                      | Record in key-insights only                 |

## Constraints
- `esp-hal =1.2.2` exact pin, MSRV 1.95 on the hal crate, separate `target/hal`, chip features C3/C6/ESP32/S3 as in parity v1.
- The esp-hal sleep API is `#[instability::unstable]`; every claim is pinned to 1.2.2.
- C3 LP pads are GPIO0-5; other pads return `WakeConfigError::NoLowPowerPath`.
- After a rejected sleep esp-hal does not restore every pad; wake pins are re-armed on every attempt.
- `WakeCause::Gpio(mask)` is assembled from `caused_wakeup()` on the registered pins; the mask shape comes from the C3 wake feature.
- Depends on the hal crate scaffold from parity v1 and on `stoker` being `no_std` with its error enum.
- Light sleep, async sleep, PM locks, ULP, and S3/ESP32 sleep on esp-hal are out of scope.

## Open Questions
- [ ] Does the hal manager borrow `&mut Input` wake pins at sleep time, or own them (ADR spike)?
- [ ] Does esp-hal 1.2.2 deep sleep keep RTC fast memory on the C3 given `rtc_fastmem_pd_en` is never applied (bench; decides the retention feature's esp-hal half)?
- [ ] Which esp-rs issue numbers track the three findings once filed?

## State
- [x] Design approved
- [ ] ADR `docs/adr/00N-hal-neutral-sleep-contract.md` accepted and linked from parity v1
- [ ] Three esp-rs issues filed (GPIO light-sleep-only doc, `rtc_fastmem_pd_en` unused on C3, C3 metadata lists Touch/Ulp)
- [ ] `rustyfarian-esp-hal-power` sleep module: `SleepManager`, `WakeCauseSource`, `try_sleep`
- [ ] Example `hal_c3_deep_sleep_gpio`, twin of the IDF example, same wiring
- [ ] Hardware-validated on the C3 DevKitM-1: GPIO4 wake, one rejection, timer fallback
- [ ] Tests passing (`just verify`, hal check recipe)
- [ ] CHANGELOG `[Unreleased]`: Added (hal sleep module)

## Session Log
- 2026-10-12 — Feature doc created via /feature dialog from the spookhouse request
