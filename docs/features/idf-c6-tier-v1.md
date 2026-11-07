---
gate: none
desk-work: available
---

# Feature: ESP-IDF ESP32-C6 Tier v1

The ESP-IDF tier builds and links for the ESP32-C6 (`riscv32imac-esp-espidf`), matching the C3 and S3 deep-sleep and retention surface.
Plan item 1 of the tier-parity wave; the esp-hal C6 counterpart is in `esp-hal-sleep-tier-v1.md` and `esp-hal-parity-v1.md`.
Capability status: `docs/features/esp-hal-parity-v1.md` § capability table.

## Decisions
|                                                                  Decision | Reason                                                                           | Rejected Alternative                           |
|--------------------------------------------------------------------------:|:---------------------------------------------------------------------------------|:-----------------------------------------------|
| `cfg(esp32c6)` selects `ChipPolicy::Esp32C6` (guard `Never`, mask `0xFF`) | The C6 has EXT1 and no pre-read path; the stoker table already defines this row  | Keep `ChipPolicy::Other` (mask check skipped)  |
|                     Build script only adds `rustc-check-cfg=cfg(esp32c6)` | esp-idf-sys 0.38.1 already emits the bare cfg; no MCU or triple parsing          | Parse `MCU` or the target triple in `build.rs` |
|                          C6 deep-sleep example uses GPIO2 as the wake pin | GPIO4-7 are JTAG, GPIO8/9 strapping or LED; GPIO2 not flagged by esp-metadata    | GPIO4 (JTAG) or GPIO8/9 (strapping)            |
|                              C6 example is leaner than the C3 one [^lean] | `Esp32C6` never pre-reads, so the C3 bench scaffolding does not apply            | Port the C3 example unchanged                  |
|      `[target.riscv32imac-esp-espidf]` block in both cargo configs [^cfg] | Same time64 setting as the other IDF targets; `.dist` is the tracked copy [^git] | Target block only in the local file            |
|                                CI job `check-c6` pinned to `ubuntu-24.04` | The 26.04 image breaks ESP-IDF tool self-checks (see key-insights CI section)    | `ubuntu-latest`                                |

[^lean]: No awake probe, no pin-read window, no `C3_*` knobs.
[^cfg]: The block (ldproxy, `espidf_time64`) lives in `.cargo/config.toml.dist` and `.cargo/config.toml`.
[^git]: `.cargo/config.toml` is gitignored.

## Constraints
- Retention (`RetainedCell`) needs the opt-in `critical-section-impl` provider; `idf_c6_rtc_retention` enables it through its `[[example]]` `required-features`.
- The C6 guard `Never` is provisional: a held EXT1 pin is assumed to be handled by the hardware reject; this is unproven.
- Build directory: `target/idf`; a cold C6 check takes about 20 minutes without the RAM disk.

## Open Questions
- [ ] Does a held GPIO2 at its wake level return `Rejected` on the C6 (keep `Never`), or does the chip wake at once and boot-loop (switch to `Always`, one line in `esp_sleep.rs`)?
- [ ] Does `.rtc_noinit` (0x50000028) survive deep sleep on the C6, as it does on the C3?

## State
- [x] Design approved
- [x] `build.rs` `check-cfg` and `esp_sleep.rs` `chip_policy()` C6 arm
- [x] `[target.riscv32imac-esp-espidf]` in `.cargo/config.toml.dist` and `.cargo/config.toml`
- [x] Examples `idf_c6_deep_sleep_gpio` and `idf_c6_rtc_retention` link in release (`just ci-c6`)
- [x] `just check-c6` and `just ci-c6` recipes; `build-example.sh`, `run-example.sh` and `ensure-bootloader.sh` accept `c6`
- [x] CI job `check-c6` in `.github/workflows/rust.yml`
- Hardware checks for the final pass (not a gate):
- [ ] C6-DevKitC-1: the boot line prints a reset reason and the BENCH boot line.
- [ ] C6-DevKitC-1: with GPIO2 held at its wake level, record the `try_sleep` result and the next boot's reset and wake cause (decides `Never` or `Always`).
- [ ] C6-DevKitC-1: timer wake with `C6_SLEEP_TIMER_MS=5000` gives wake cause `Timer` and reset `DeepSleep`.
- [ ] C6-DevKitC-1: EXT1 wake on GPIO2 gives `Ext1` with fired-pin mask `0x4`.
- [ ] C6-DevKitC-1: `idf_c6_rtc_retention` with `C6_RET_ACTION=cycle` keeps the session id across deep sleep, software reset, panic and watchdog reset.
- [ ] Flash path: `just flash idf_c6_rtc_retention` goes through `run-example.sh` with espflash chip `esp32c6` and the cached v5.3.3 bootloader.
- [ ] Expected-output sections in both C6 examples are marked not yet bench-verified.

## Session Log
- 2026-11-04 — Feature doc created for tier-parity plan item 1; hardware steps moved to the final pre-release pass
