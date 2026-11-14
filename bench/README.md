# Bench

Guided, rerunnable hardware checks for both power tiers ([ADR-005](../docs/adr/005-adopt-rustyfarian-bench.md)).
The runner is [`rustyfarian-bench`](https://github.com/datenkollektiv/rustyfarian-bench) (Python + uv).
It needs `uv`, `espflash` and the ESP-RS toolchain (`espup`); `bench-build` and `bench-flash` put the Xtensa linker on `PATH` themselves, so no `source ~/export-esp.sh` is needed.
The per-board READMEs are the record of what is tested on hardware: one row per scenario, with what a pass proves.

| Board                                  | Directory                        | Last full pass |
|:---------------------------------------|:---------------------------------|:---------------|
| ESP32-C3-DevKitM-1 (ESP32-C3-MINI-1)   | [c3-mini1/](c3-mini1/)           | 2026-11-08     |
| Heltec WiFi LoRa 32 V3 (S3)            | [s3-heltec/](s3-heltec/)         | 2026-11-10     |
| Adafruit ESP32 Feather V2              | [esp32-feather/](esp32-feather/) | 2026-11-10     |
| CrowPanel 1.28" HMI (S3)               | [s3/](s3/)                       | 2026-11-10     |
| ESP32-C6-DevKitC-1                     | `c6/`                            | open           |

A full pass is every scenario of the board in both modes, on both tiers where a twin exists.

## Running a check

From this repo's root; `just bench` runs `rustyfarian-bench` v0.5.0 from GitHub, or from a local checkout if `RUSTYFARIAN_BENCH` points at one:

```shell
just bench c3-mini1 retention
just bench hal_c3_rtc_retention
just bench s3-heltec rail-gate --no-build
just bench c3-mini1
just bench-list
just bench-detect
```

To run unreleased runner changes from the sibling checkout:

```shell
RUSTYFARIAN_BENCH=../rustyfarian-bench just bench c3-mini1 pir-wake
```

A board's checks split into steps the log can judge and steps that need a person (a `confirm`, or `attended = true` such as the PIR wave).
Run the long, hands-off part first and walk away, then the short attended part:

```shell
just bench c3-mini1 --unattended
just bench c3-mini1 --attended
```

Each result records its `mode`, so a pass in one mode is not a full pass; a release needs both.

The runner builds first, waits for the board (plug it in when asked), flashes, resets, and guides each step with an automatic verdict.
A scenario runs the hal example first, then offers its idf twin.
The Heltec V3 and the CrowPanel are both ESP32-S3; `bench.toml` pins each by its MAC, so both can stay attached without `--port`.
Detection probes every candidate port with espflash, which resets whatever is on it, so pin `--port` while the PPK2 or another board is attached.

## Layout

- `bench.toml` — build/flash commands and the boards (chip, optional MAC).
- `<board>/<check>.toml` — one scenario: the examples it runs and the shared steps.
- `<board>/logs/` — captured serial output per run (git-ignored).
- `<board>/results.jsonl` — one JSON line per example run (git-ignored: it records board MACs).

The commands call `just bench-build NAME` and `just bench-flash NAME PORT`.
They dispatch on the name: `idf_*` through `build-example` / `flash`, `hal_<chip>_*` through `build-example-hal-<chip>` / `flash-hal-<chip>`.
Scenarios use the default build of each example, or a **bench variant**.
`NAME--VARIANT` in a scenario's `examples` makes `bench-build` and `bench-flash` build and flash NAME with `BENCH_VARIANT=VARIANT`.
Examples that support variants read it at build time and fail the build on an unknown value; results and logs keep the full `NAME--VARIANT`.
The older per-example switches (`C3_SLEEP_TIMER_MS`, `C3_RET_ACTION`, …) do not reach the runner until an example maps them to a variant.

## Relation to the other bench tools

- `just bench-c3` (`scripts/c3-bench.py`) drives the two-board C3 sleep cases with the WROOM stimulus.
- The `just bench-c3` operating guide is [c3-devkitm1-sleep-bench-plan-v1](../docs/bench/c3-devkitm1-sleep-bench-plan-v1.md).
- `just serial-capture PORT SECONDS` records a timestamped serial log without touching DTR/RTS, for a hand-run example that no scenario covers.
- This bench covers the default builds and the bench variants: one guided check per facet, rerunnable and comparable across tiers.

## Recording a result

- Record any non-obvious gotcha in `docs/project-lore.md`.
