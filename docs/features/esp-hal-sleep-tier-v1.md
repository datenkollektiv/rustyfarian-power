---
gate: none
desk-work: available
---

# Feature: esp-hal Sleep Tier (C3-first) v1

Sleep and wake-cause support in `rustyfarian-esp-hal-power`, scoped to the ESP32-C3 first and extended to the S3, ESP32 and C6.
Requested by rustbox-spookhouse for its esp-hal prop; the consumer requirements and the whole contract are in [ADR-001](../adr/001-hal-neutral-sleep-contract.md).
Bench results per board are in `bench/<board>/README.md`; the two-board C3 driver is `docs/bench/c3-devkitm1-sleep-bench-plan-v1.md`.

## Decisions
| Decision | Reason | Rejected Alternative |
|---:|:--|:--|
| Contract fixed in ADR-001 before code | Trait shape, pin ownership, errors and pre-validation decided in one place | Driver-only port with an ad-hoc API |
| Rejectable sleep over `sleep_deep_with_rejection` | Same `try_sleep` contract as the IDF tier | `sleep_deep` only, panics on a held pin |
| Wake pins borrowed as `&mut Input` per attempt via `bind_wake_pins` | Compiles on esp-hal 1.2.2; the firmware keeps polling the pin | Manager-owned pins |
| `EspHalWakeCauseSource::capture(&[&Input])` at boot, before any `try_sleep` | `caused_wakeup()` is per pin and a rejected sleep rewrites the record | Read the cause lazily |
| File the three esp-hal 1.2.2 findings as esp-rs issues | Upstream decides; the links become external dependencies | Record them locally only |

## Constraints
- `esp-hal =1.2.2` exact pin; the sleep API is `#[instability::unstable]`, so every claim is pinned to that version.
- C3 LP pads are GPIO0-5; other pads return `WakeConfigError::NoLowPowerPath`.
- Wake pins need an external pull against the wake level; esp-hal adds none and a floating pad wakes immediately.
- After a rejected sleep the wake pins are released; other digital pads are the firmware's to reconfigure (ADR-001 § 4).
- Arming, sleep and cleanup run inside one `critical_section::with`; a custom GPIO handler must not `unlisten` a bound pin (ADR-001 § 4b).
- `MIN_TIMER_MS` (1 ms) and `POST_ARM_MARGIN_US` (200 µs) are provisional until the `short-timer-sweep` bench sets them (ADR-001 § 4c).
- Light sleep, async sleep, PM locks and ULP are out of scope.

## State
- [ ] ADR-001 acceptance: `short-timer-sweep` sets the timer constants (needs the PPK2); § 4b-4d cases `wake-cause-survives-rejection`, `held-level-custom-handler`, `rejection-retry-loop`, `foreign-wake-pin` (`scripts/c3-bench.py` H2-H8).
- [ ] File the three esp-rs issues and record their numbers: GPIO wake documented as light-sleep only, `rtc_fastmem_pd_en` unused on the C3, C3 metadata lists `Touch` and `Ulp`.
