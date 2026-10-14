---
gate: none
desk-work: available
---

# Feature: ESP32-S3 Deep Sleep with GPIO Wake v1

S3 deep sleep with a GPIO wake on both tiers, upstream request U4 from rustyfarian-knob.
Request copy: `review-queue/feature-request-esp-hal-sleep-esp32s3.md`; follows [esp-hal sleep tier v1](esp-hal-sleep-tier-v1.md).

## Decisions
|                                                Decision | Reason                                                             | Rejected Alternative                       |
|--------------------------------------------------------:|:-------------------------------------------------------------------|:-------------------------------------------|
| Promote the ADR-001 spike to an exported `sleep` module | A hal example needs public API; the spike is private and C3-only   | cfg arms on the spike only, no hal example |
|                      One module with C3 and S3 cfg arms | Keeps the ADR's C3-first order; the S3 is a second arm, not a fork | S3-only module                             |
|                   Keep the `bind_wake_pins` binding API | esp-hal 1.2.2 assigns S3 pins to ext0/ext1/per-pin internally      | S3-specific ext1 binding API               |
|                   S3 wake-pin mask from `stoker` (0-21) | Same validation and `ForeignWakePin` scan range on both tiers      | Hard-coded C3 `0x3F` set                   |
|                IDF tier: example only, no driver change | EXT1 path and GPIO0-21 validation already exist (`esp_sleep.rs`)   | New S3-specific IDF arm                    |
|                          ADR-001 amended to C3+S3 scope | Contract is chip-neutral; S3 facts and bench results belong there  | Separate S3 ADR                            |

## Constraints
- esp-hal `=1.2.2` exact pin; every S3 claim is checked against the registry source.
- Wake source on the knob is CST816S touch INT on GPIO5, active-low, `Pull::Floating`.
- The CST816S needs `LCD_3V3` (GPIO1) on and RST (GPIO13) high, held through sleep, or INT idles LOW.
- The bench CrowPanel's console is native USB-Serial-JTAG, not a CH340; it drops off the bus in deep sleep.
- GPIO41/42/45 (encoder, button) are not RTC pads and must be rejected with `GpioMaskOutOfRange`.
- `ext1` has one level for its whole mask; mixed levels keep the LP peripherals powered.
- Bench only on the CrowPanel 1.28" ESP32-S3; the C3 hal bench stays open in esp-hal-sleep-tier-v1.
- Release is lockstep with `stoker` first and decided separately; nothing is committed without asking.

## Open Questions
- [x] Does the S3 refuse deep sleep with the ext1 pin at its wake level? Yes, on both tiers; no pre-check needed.
- [x] Does GPIO5 keep its idle level in deep sleep? Yes, held HIGH, once `LCD_3V3` and touch RST are on and held.
- [x] Normalise the wake cause? Per chip: S3 reports `Ext1(mask)` on both tiers, C3 `Gpio(mask)` (ADR-001 § 1).
- [ ] A resting finger pulses INT and loops wakes; which CST816S IRQ mode or firmware wait should the knob use?

## State
- [x] Design approved
- [x] esp-hal `sleep` module exported with C3 and S3 arms; spike removed
- [x] Example builds for the hal crate (`build-example-hal-s3`, `run-hal-s3`)
- [x] `hal_s3_deep_sleep_gpio` and `idf_s3_deep_sleep_gpio` examples
- [x] ADR-001 amended to C3+S3
- [x] S3 bench doc; touch wake 3/3, timer, held finger and held level per tier
- [x] README, CHANGELOG `[Unreleased]`, `maintenance-plan.md` hardware row
- [x] Tests passing (`just verify`, `just check-hal`, `just clippy-hal`)

## Session Log
- 2026-10-14 — Feature doc created via /feature from the knob's U4 request
- 2026-10-14 — Implemented both tiers; CrowPanel bench passed W1/W2/H1 on IDF and esp-hal; open questions answered
- 2026-10-14 — Bench tooling removed; examples reduced to lightweight demos; bench doc kept as a record
