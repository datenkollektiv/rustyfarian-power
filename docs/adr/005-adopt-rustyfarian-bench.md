# ADR-005: Adopt rustyfarian-bench for on-device checks

## Status
Accepted (2026-11-07).
Follows `rustyfarian-peripherals`, `rustyfarian-ws2812` and `rustyfarian-knob` (knob ADR 016), which run their checks through the same runner.

## Context
- The power crates are validated on-device by hand: `just run` plus reading the console, `just serial-capture` for the unattended R-cases, and `just bench-c3` for the two-board C3 sleep cases.
- Only the bench docs under `docs/bench/` record which case passed on which tier; a rerun means re-reading a runbook.
- The rustyfarian family now has `rustyfarian-bench`, a guided runner (Python + uv, released as git tags on GitHub).
  A consumer describes boards and checks as TOML under `bench/`; the runner builds, finds the board, flashes, resets, captures serial and judges each step against regexes or an operator answer.
- Most examples here come in `hal_*` / `idf_*` pairs with stable log lines (`RET`, `BENCH case=rail`, `Battery:`, `Wake cause:`), which fit one scenario running several examples through shared steps.
- The runner calls one build and one flash command per example name, but the two tiers build and flash through different recipes and target dirs.

## Decision
1. On-device checks of the default example builds are written as `rustyfarian-bench` scenarios under `bench/<board>/`, one per facet, each running the hal example first and then its idf twin.
2. `bench/bench.toml` calls two new recipes, `just bench-build NAME` and `just bench-flash NAME PORT`.
   They dispatch on the name prefix to the existing `build-example` / `flash` and `build-example-hal-<chip>` / `flash-hal-<chip>` recipes; the port reaches espflash through `ESPFLASH_PORT`.
3. `just bench`, `just bench-list` and `just bench-detect` match the sibling repos and run a pinned release through `uvx --from git+https://github.com/datenkollektiv/rustyfarian-bench@v0.3.0`.
   Bumping the tag is how this repo opts into a new runner.
   `RUSTYFARIAN_BENCH=<path>` switches to a local checkout for bench development.
4. Run logs and `results.jsonl` stay git-ignored, because results record board MACs and this repository is public; `bench.toml` pins no MAC.
5. The first pass covers boards whose console survives deep sleep (UART bridge): C3 retention and PIR wake, Heltec V3 retention, rail gate and battery, Feather V2 battery.
6. `just bench-c3` and the `serial-capture` R-cases stay; they need build-time switches or a stimulus board that the runner cannot drive.
7. Build-time cases run as bench variants: a scenario names `NAME--VARIANT`.
   `bench-build` and `bench-flash` export `BENCH_VARIANT=VARIANT` for both steps (so the flash step's rebuild uses the same variant).
   The example maps the value to its configuration, failing the build on an unknown one.
   This lets switch-dependent cases run through the runner.

## Consequences
- Hardware validation of the default builds becomes rerunnable and comparable across tiers.
- The verdict is automatic where serial output proves it, and an explicit operator answer where it cannot (battery voltage against a meter).
- Bench runs depend on `uv` and network access to GitHub on first use (then the uv cache); builds, `just verify` and CI are unaffected.
- Build-time switches (`C3_SLEEP_TIMER_MS`, `C3_RET_ACTION`, `S3_R2`, …) reach the runner only through decision 7; until an example maps a switch to a variant, its cases keep their existing tools.
- Scenario regexes couple to example log lines; changing an example's log text means updating its scenario.

## Alternatives Considered
| Alternative                               | Pros                           | Cons                                          | Why Rejected                                          |
|:------------------------------------------|:-------------------------------|:----------------------------------------------|:------------------------------------------------------|
| Extend `scripts/c3-bench.py` to all chips | no sibling dependency          | one more bespoke driver per board             | the family converged on one runner                    |
| `bench.toml` calls `just flash` directly  | no new recipes                 | `just flash` handles `idf_*` only             | hal examples need the per-chip recipes                |
