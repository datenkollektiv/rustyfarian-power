# Bench

Guided, rerunnable hardware checks for both power tiers ([ADR-005](../docs/adr/005-adopt-rustyfarian-bench.md)).
The runner is [`rustyfarian-bench`](https://github.com/datenkollektiv/rustyfarian-bench) (Python + uv).
It needs `uv`, `espflash` and the ESP-RS toolchain (`espup`); `bench-build` and `bench-flash` put the Xtensa linker on `PATH` themselves, so no `source ~/export-esp.sh` is needed.

| Board                          | Directory          | State       |
|:-------------------------------|:-------------------|:------------|
| ESP32-C3-MINI-1 / DevKitM-1    | [c3/](c3/)         | passed 2026-11-08 (DevKitM-1) |
| Heltec WiFi LoRa 32 V3 (S3)    | [heltec/](heltec/) | in progress |
| Adafruit ESP32 Feather V2      | [esp32/](esp32/)   | passed 2026-11-09 (Feather V2); battery meter-confirm still to run attended |
| CrowPanel 1.28" HMI (S3)       | `s3/`              | open        |
| ESP32-C6-DevKitC-1             | `c6/`              | open        |

## Running a check

From this repo's root; `just bench` runs `rustyfarian-bench` v0.3.0 from GitHub, or from a local checkout if `RUSTYFARIAN_BENCH` points at one:

```shell
just bench c3 retention
just bench hal_c3_rtc_retention
just bench heltec rail-gate --no-build
just bench c3
just bench-list
just bench-detect
```

To run unreleased runner changes from the sibling checkout:

```shell
RUSTYFARIAN_BENCH=../rustyfarian-bench just bench c3 pir-wake
```

A board's checks split into steps the log can judge and steps that need a person (a `confirm`, or `attended = true` such as the PIR wave).
Run the long, hands-off part first and walk away, then the short attended part:

```shell
just bench c3 --unattended
just bench c3 --attended
```

Each result records its `mode`, so a pass in one mode is not a full pass; a release needs both.

The runner builds first, waits for the board (plug it in when asked), flashes, resets, and guides each step with an automatic verdict.
A scenario runs the hal example first, then offers its idf twin.
The Heltec V3 and the CrowPanel are both ESP32-S3: with both attached, pin the board with `--port`.
Detection probes every candidate port with espflash, which resets whatever is on it, so pin `--port` while the PPK2 or another board is attached.

## Layout

- `bench.toml` — build/flash commands and the boards (chip, optional MAC).
- `<board>/<check>.toml` — one scenario: the examples it runs and the shared steps.
- `<board>/logs/` — captured serial output per run (git-ignored).
- `<board>/results.jsonl` — one JSON line per example run (git-ignored: it records board MACs).

The commands call `just bench-build NAME` and `just bench-flash NAME PORT`, which dispatch on the name: `idf_*` through `build-example` / `flash`, `hal_<chip>_*` through `build-example-hal-<chip>` / `flash-hal-<chip>`.
Scenarios use the default build of each example, or a **bench variant**: `NAME+VARIANT` in a scenario's `examples` makes `bench-build` and `bench-flash` build and flash NAME with `BENCH_VARIANT=VARIANT`.
Examples that support variants read it at build time and fail the build on an unknown value; results and logs keep the full `NAME+VARIANT`.
Variants so far: `anylow`, `anylow-multi` on `{hal,idf}_esp32_deep_sleep_gpio`.
The older per-example switches (`C3_SLEEP_TIMER_MS`, `S3_R2`, …) do not reach the runner until an example maps them to a variant.

## Relation to the other bench tools

- `just bench-c3` (`scripts/c3-bench.py`) stays the driver for the two-board C3 sleep cases with the WROOM stimulus (`docs/bench/c3-devkitm1-sleep-bench-plan-v1.md`).
- `just serial-capture` stays the tool for the unattended R-cases that need build-time switches (`docs/bench/`).
- This bench covers the default builds: one guided check per facet, rerunnable and comparable across tiers.

## Recording a result

- Record any non-obvious gotcha in `docs/project-lore.md`.
