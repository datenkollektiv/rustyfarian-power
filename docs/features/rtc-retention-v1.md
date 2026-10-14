---
gate: none
desk-work: available
---

# Feature: RTC Retention Record v1

A validity-checked, `no_std`, allocation-free retention envelope in `stoker` with placement in both tiers, so a few words survive deep sleep and ordinary resets without each firmware writing its own magic-and-checksum record.
Requested by rustbox-spookhouse (`docs/features/upstream-sleep-wake-requests-v1.md` in that repo, filed 2026-10-12).
Verified 2026-10-12: the C3 has 8 KB RTC fast memory at `0x50000000` and IDF aliases `.rtc.data` onto it; `.rtc_noinit` is `NOLOAD`.

## Decisions
|                                                                                                                  Decision | Reason                                                                                                                                                                                                      | Rejected Alternative                                                               |
|--------------------------------------------------------------------------------------------------------------------------:|:------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|:-----------------------------------------------------------------------------------|
|                                                                            Reset survival: IDF placement in `.rtc_noinit` | Watchdog and software resets keep the record; a bonus the consumer wants                                                                                                                                    | `.rtc.data` plus `empty()`, deep sleep only                                        |
|                                               Envelope of integers and a byte array with magic, version, length, checksum | Arbitrary bits after power loss are never UB                                                                                                                                                                | Placing an arbitrary `T` in RTC memory                                             |
|                                                            `Retainable` serialises via `write`/`read` into the byte array | Checksum never covers padding bytes                                                                                                                                                                         | `bytemuck`-style plain-old-data cast                                               |
|                                                  New `ResetReasonSource` trait and `ResetReason` enum with a `Noop*` mock | Resets and sleep stay separate concerns; additive to `stoker`                                                                                                                                               | Splitting `WakeCause::PowerOn` into reset variants                                 |
|                                              `load` treats `PowerOn` and `Brownout` as invalid, others trust the checksum | Brownout may leave a partial record that still checksums by chance                                                                                                                                          | Checksum only                                                                      |
|                                                  Both tiers expose the record through a `static` with interior mutability | No `static mut`; the hal tier wraps `Retained` in its own `Persistable` cell                                                                                                                                | `static mut` as in the current docs                                                |
|                                       `RetainedCell<N>` lives in `stoker`, not in each tier (2026-10-12, consumer review) | One sound read/write path (`MaybeUninit` + volatile + fences, reset gate before the read) instead of one per tier; tiers only add the section attribute                                                     | Per-tier cells as first designed                                                   |
|                                      `ResetReason` gains `Panic` and is `#[non_exhaustive]` (2026-10-12, consumer review) | A panic mid-`store` and a clean restart are different events; variants can be added after 0.3.0 without a breaking release                                                                                  | Folding panic into `Software`; exhaustive enum                                     |
|                                                                               Ships in the same 0.3.0 as the C3 wake work | One release train, one CHANGELOG cut                                                                                                                                                                        | 0.2.1 for the IDF half                                                             |
| Checksum is table-less CRC-32 (IEEE 802.3) over little-endian `magic`, `version`, `len` and the first `len` payload bytes | Catches torn writes and runs of zeros; all-zero and all-`0xFF` memory already fails the magic check; negligible cost for < 100 bytes at boot                                                                | Fletcher-32 / Adler-32 (weak on zero runs), a `crc` dependency (`stoker` has none) |
|                                                                                                          No A/B slot pair | A reset during `store` leaves a record the checksum rejects and the consumer falls back to fresh state, the same outcome a stale second slot gives; the consumer may keep two statics if it wants last-good | Two slots with a sequence number, doubling the RTC footprint                       |
|                                                                      `Retainable` carries a required `const VERSION: u16` | The envelope cannot know the payload layout; a default lets schemas drift silently                                                                                                                          | Envelope-level version only; defaulted const                                       |
|                                                              `store` returns `Result<(), StoreError { Length, Payload }>` | `Invalid` holds `PowerOn`/`Brownout`, which `store` can never return; a typed error matches the house `SleepError` style                                                                                    | `Option<()>` as in the request                                                     |
|                                                                      `Invalid` gains `Payload` and is `#[non_exhaustive]` | A payload decoder may reject bytes that checksum fine (out-of-range discriminant)                                                                                                                           | Folding it into `Checksum`                                                         |
|                          Fixed-width integers (`u8`–`u64`, `i8`–`i64`) implement `Retainable` with a little-endian layout | A boot counter needs no custom impl; replaces the `static mut BOOT_COUNT: u32` recipe directly                                                                                                              | Payload impls only in the consumer                                                 |
|         Modules `stoker::retain` (envelope) and `stoker::reset` (`ResetReason*`), all types re-exported at the crate root | Resets and sleep stay separate modules, mirroring `sleep.rs`                                                                                                                                                | One `rtc` module                                                                   |
|                                        `Retained` is `#[repr(C)]` with field order `magic, checksum, version, len, bytes` | Stable layout across firmware builds; only trailing padding                                                                                                                                                 | Natural field order with interior padding before `checksum`                        |

## Constraints
- `N >= T::SIZE` is checked at runtime (`Invalid::Length`); stable Rust cannot express it in the type.
- A `static` initialiser in `.rtc_noinit` is never applied; `empty()` is only for `.rtc.data` placement.
- esp-riscv-rt zeroes `.rtc_fast.persistent` only on power-on reset and only with its `rtc-ram` feature.
- The esp-hal half is unproven until the `rtc_fastmem_pd_en` finding is bench-checked (esp-hal sleep tier doc).
- NVS or flash persistence, full `PresenceSession` retention, and a pending-report queue are out of scope.
- Every bit pattern is a valid `Retained`, but Rust still treats never-written `NOLOAD` memory as uninitialised; `RetainedCell` reads through `MaybeUninit` and volatile access so the compiler cannot constant-fold the initialiser it sees in source.
- `RetainedCell` is not synchronised (riscv32imc has no CAS for a lock, and `stoker` stays dependency-free); a `store` overlapping another access on the same cell is a data race, so firmware uses it from one context.
- esp-hal's `#[ram(unstable(rtc_fast, persistent))]` asserts `esp_hal::Persistable` on the static's type; the orphan rule means the hal tier must wrap `RetainedCell` in its own newtype and `unsafe impl Persistable` for it.
- Two payload types with equal `VERSION` and `SIZE` (for example `u32` and `i32`) load each other's bytes; the envelope cannot tell them apart.
- The `.rtc.data` `static mut` recipe in `sleep.rs`, `esp_sleep.rs` and `docs/hardware-setup.md` is replaced by the envelope.

## Consumer Requirements

The rustbox-spookhouse firmware retains approximately 60 bytes across deep sleep:
- Session id (`u32`) — generated per cold boot, mixes the MAC, a retained boot counter, and `esp_random`
- Report sequence (`u32`) — incremented per transmitted report, not per boot
- Encounter counter (`u32`) — counts PIR triggers in the current session
- Pending report slot (`Option<PendingReport>`) — serialised report awaiting transmission
- Session snapshot (`SessionState` from `tamer::presence`) — determines whether a watchdog reset restarts the warm-up period or resumes it

The envelope must carry this payload through GPIO and timer wake events, software and watchdog resets (bonus), and be rejected cleanly after power-on or brownout so the receiver deduplicates on session id + sequence.
`new_session_id()` cannot run at every boot because the RNG is weak with the radio off; it depends on the retained counter.

## Open Questions
- [x] Fixed little-endian encoding, checksum algorithm, and whether an A/B slot pair is needed against torn writes — decided 2026-10-12, see Decisions.
- [ ] Does `#[link_section = ".rtc_noinit.<name>"]` place a Rust static at or above `0x50000000` on the C3 (check the map file)?
- [x] Module name in `stoker` — `retain` and `reset`.
- [ ] Helper for plain-data `Retainable` payloads (a small macro writing the little-endian `write`/`read` pair) — deferred until the spookhouse impl shows what repeats; manual impls stay the safe default.

## State
- [x] Design approved
- [x] `stoker`: envelope, `Retainable`, `Invalid`, `ResetReasonSource`, `ResetReason`, `Noop*` mock, host tests
- [x] `stoker`: `RetainedCell` (volatile, fenced, reset-gated), `ResetReason::Panic` + `#[non_exhaustive]`, magic-first `store`, torn/layout/large/garbage tests
- [ ] IDF tier: `esp_reset_reason()` mapping, `.rtc_noinit` placement, cell type, example
- [ ] esp-hal tier: `reset_reason(Cpu::ProCpu)` mapping, `#[ram(unstable(rtc_fast, persistent))]` cell, example
- [ ] Hardware check on the C3 for both tiers: GPIO wake, timer wake, watchdog reset, bench brownout, power cycle
- [ ] Tests passing (`just verify`)
- [ ] CHANGELOG `[Unreleased]`: Added (envelope, `ResetReasonSource`); docs drop the `static mut` recipe — entries and `sleep.rs` done; `esp_sleep.rs` and `docs/hardware-setup.md` follow with the IDF placement

## Session Log
- 2026-10-12 — Feature doc created via /feature dialog from the spookhouse request
- 2026-10-12 — `stoker` half landed: `retain.rs` (`Retained<N>`, `Retainable`, `Invalid`, `StoreError`, CRC-32), `reset.rs` (`ResetReason`, `ResetReasonSource`, `NoopResetReasonSource`), 32 host tests covering every acceptance case plus the CRC-32 check value; encoding and checksum decided; `sleep.rs` recipe replaced. IDF and esp-hal placement wait on the `check-c3` target and the hal crate scaffold.
- 2026-10-12 — Consumer review (rustbox-spookhouse) applied: `RetainedCell` moved into `stoker`, `ResetReason::Panic` + `#[non_exhaustive]`, `store` clears the magic first; 12 more host tests (torn store, layout, 1 KB envelope, 10 k garbage records, cell behaviour). Hal-tier `Persistable` newtype noted under Constraints.
- 2026-10-12 — Second review pass: `RetainedCell` docs gain an explicit concurrency contract and a `new()` vs `empty()` note; `reset.rs` carries the single `ResetReason` mapping table for IDF and esp-hal and states that `Panic` is best-effort (esp-hal reports `Software`); new integration test `tests/retention_boot_cycle.rs` drives a static cell through cold boot, ten sleep cycles, watchdog/software/panic/other resets and a brownout. Plain-data helper deferred (Open Questions).
- 2026-10-14 — Folded spookhouse request into this doc; `Consumer Requirements` section captures payload composition (60 bytes, session id, sequence, counter, pending report, session snapshot) and cold-boot id generation contract; review-queue copies retired.
