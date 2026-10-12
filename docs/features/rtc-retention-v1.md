---
gate: none
desk-work: available
---

# Feature: RTC Retention Record v1

A validity-checked, `no_std`, allocation-free retention envelope in `stoker` with placement in both tiers, so a few words survive deep sleep and ordinary resets without each firmware writing its own magic-and-checksum record.
Requested by rustbox-spookhouse (session id, report sequence, presence snapshot); request copy in `review-queue/rustyfarian-power-rtc-retention-v1.md`.
Verified 2026-10-12: the C3 has 8 KB RTC fast memory at `0x50000000` and IDF aliases `.rtc.data` onto it; `.rtc_noinit` is `NOLOAD`.

## Decisions
|                                                                     Decision | Reason                                                                       | Rejected Alternative                               |
|-----------------------------------------------------------------------------:|:-----------------------------------------------------------------------------|:---------------------------------------------------|
|                               Reset survival: IDF placement in `.rtc_noinit` | Watchdog and software resets keep the record; a bonus the consumer wants     | `.rtc.data` plus `empty()`, deep sleep only        |
|  Envelope of integers and a byte array with magic, version, length, checksum | Arbitrary bits after power loss are never UB                                 | Placing an arbitrary `T` in RTC memory             |
|               `Retainable` serialises via `write`/`read` into the byte array | Checksum never covers padding bytes                                          | `bytemuck`-style plain-old-data cast               |
|     New `ResetReasonSource` trait and `ResetReason` enum with a `Noop*` mock | Resets and sleep stay separate concerns; additive to `stoker`                | Splitting `WakeCause::PowerOn` into reset variants |
| `load` treats `PowerOn` and `Brownout` as invalid, others trust the checksum | Brownout may leave a partial record that still checksums by chance           | Checksum only                                      |
|     Both tiers expose the record through a `static` with interior mutability | No `static mut`; the hal tier wraps `Retained` in its own `Persistable` cell | `static mut` as in the current docs                |
|                                  Ships in the same 0.3.0 as the C3 wake work | One release train, one CHANGELOG cut                                         | 0.2.1 for the IDF half                             |

## Constraints
- `N >= T::SIZE` is checked at runtime (`Invalid::Length`); stable Rust cannot express it in the type.
- A `static` initialiser in `.rtc_noinit` is never applied; `empty()` is only for `.rtc.data` placement.
- esp-riscv-rt zeroes `.rtc_fast.persistent` only on power-on reset and only with its `rtc-ram` feature.
- The esp-hal half is unproven until the `rtc_fastmem_pd_en` finding is bench-checked (esp-hal sleep tier doc).
- NVS or flash persistence, full `PresenceSession` retention, and a pending-report queue are out of scope.
- The `.rtc.data` `static mut` recipe in `sleep.rs`, `esp_sleep.rs` and `docs/hardware-setup.md` is replaced by the envelope.

## Open Questions
- [ ] Fixed little-endian encoding, checksum algorithm, and whether an A/B slot pair is needed against torn writes.
- [ ] Does `#[link_section = ".rtc_noinit.<name>"]` place a Rust static at or above `0x50000000` on the C3 (check the map file)?
- [ ] Module name in `stoker`.

## State
- [x] Design approved
- [ ] `stoker`: envelope, `Retainable`, `Invalid`, `ResetReasonSource`, `ResetReason`, `Noop*` mock, host tests
- [ ] IDF tier: `esp_reset_reason()` mapping, `.rtc_noinit` placement, cell type, example
- [ ] esp-hal tier: `reset_reason(Cpu::ProCpu)` mapping, `#[ram(unstable(rtc_fast, persistent))]` cell, example
- [ ] Hardware check on the C3 for both tiers: GPIO wake, timer wake, watchdog reset, bench brownout, power cycle
- [ ] Tests passing (`just verify`)
- [ ] CHANGELOG `[Unreleased]`: Added (envelope, `ResetReasonSource`); docs drop the `static mut` recipe

## Session Log
- 2026-10-12 — Feature doc created via /feature dialog from the spookhouse request
