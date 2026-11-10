---
gate: none
desk-work: available
---

# Feature: esp-hal Parity (ADC + Charging) v1

`rustyfarian-esp-hal-power` gives the bare-metal tier battery monitoring and charging detection, matching the esp-idf tier.
Sleep and wake cause live in `esp-hal-sleep-tier-v1.md` under [ADR-001](../adr/001-hal-neutral-sleep-contract.md); the parity standard is [ADR-003](../adr/003-tier-parity-contract.md).
Bench results per board are in `bench/<board>/README.md`.

## Not at parity (ADR-003 point 2)

Both tiers build for C3, C6, ESP32 and S3, and the conformance modules keep the signatures in sync.
These cells still differ or lack a bench:
- C6: every capability on both tiers is build-only; the guard `Never` is provisional (ADR-004).
- `BatteryMonitor` on C3 and C6 is compile-only on both tiers; no board preset and no example.
- `ChargingMonitor` is compile-only everywhere (HW): neither the Feather V2 nor the Heltec V3 has a charge-status GPIO.
- `PowerRailGate` runs on the Heltec V3 only; the other chips are compile-only.
- Retention and RTC clock: hal S3 has no retention example, so the Heltec bench runs the IDF tier only; IDF ESP32 is compile-only.
- `ResetReasonSource` on hal S3, hal C6 and both ESP32 tiers is built, not benched.
- Post-rejection pad state is benched on the CrowPanel S3 only; the hal S3 hardware path leaves `WIFI_FORCE_ISO` cleared.
- Multi-pin `AnyLow` on the ESP32 is refused with kind `Unsupported` on both tiers (HW, ADR-004 point 7).

## Decisions
| Decision | Reason | Rejected Alternative |
|---:|:--|:--|
| New crate `rustyfarian-esp-hal-power` | Matches the ws2812/network/peripherals tier layout | Feature flag on the IDF crate |
| `stoker` is `#![no_std]` with its own error enum | One API shape for both tiers; `anyhow` blocks no_std | `std` feature keeping `anyhow` |
| `SleepManager` gets `type Error` | The IDF impl surfaces platform errors; the enum stays the portable error | Fixed `SleepError` with IDF variants |
| Chips C3, C6, ESP32, S3 via chip features | Same four features as the sibling hal crates | ESP32 + S3 only |
| Blocking API now, `async` feature later | Mirrors ws2812/network (esp-rtos + embassy) | Blocking only |
| Deferred: ESP32 guard bench, `async`, hal `WIFI_FORCE_ISO` restore | Not needed for 0.3.0 | Block the release on them |

## Constraints
- `esp-*` crates are exact-pinned (`esp-hal =1.2.2`) in lockstep with the sibling repos.
- Embassy pins match the siblings exactly (`embassy-executor =0.10.0`, `embassy-sync =0.8.0`, `embassy-time =0.5.1`).
- The hal crate declares MSRV 1.95; `stoker` keeps MSRV 1.88.
- The esp-idf tier must not regress: `just check-all` and `just check-esp32` stay green.
- Hal builds use a separate `target/hal` dir so they do not thrash the IDF cache.
- `stoker`, `rustyfarian-esp-idf-power` and the hal crate release together as 0.3.0 (breaking: error type).

## State
- [ ] `async`: a separate `AsyncBatteryMonitor` trait or inherent methods, and whether esp-hal 1.2.2 offers async ADC.
- [ ] `BatteryConfig` presets for battery-equipped C3/C6 boards, or compile-only.
- [ ] C3-DevKitM-1: hal ADC against a known divider.
