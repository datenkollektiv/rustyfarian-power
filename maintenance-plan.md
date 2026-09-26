# Maintenance Plan

Regular maintenance workbook for `rustyfarian-power`, a Cargo workspace with two crates.
`stoker` is the pure, host-buildable core.
`rustyfarian-esp-idf-power` is the ESP-IDF (std) hardware tier for ESP32-S3 (Heltec V3) and ESP32 (Feather V2).

Covers: build verification, dependency updates, security scanning, CI/CD status, device-target compile validation, hardware validation, and toolchain freshness.

This repo has no bare-metal (`esp-hal`) tier.
An esp-hal wave in the siblings affects it only through the ESP-IDF crates that ship alongside it, the shared MSRV policy, and the CI action versions.
The sibling `rustyfarian-ws2812`, `rustyfarian-peripherals` and `rustyfarian-network` run the same protocol.
When a wave lands there first, their `maintenance-plan.md`, `CHANGELOG.md` and `docs/project-lore.md` are the reference.

## Build & Test

### Primary build gate
- `just verify` is non-modifying: fmt-check, deny, check, clippy (`-D warnings`), and host tests for `stoker`.
  Exit 0 is the audit's primary PASS signal.
- `just ci` is the CI-equivalent chain.

### Device-target compile checks
Host gates never compile the `esp-idf`-gated modules (see `docs/project-lore.md` § Build & Validation).
After any ESP-IDF crate bump, run:
- `just check-all` for ESP32-S3 (`xtensa-esp32s3-espidf`, Heltec V3)
- `just check-esp32` for ESP32 (`xtensa-esp32-espidf`, Feather V2)
- `just clippy-all`
- one `just build-example <name>` per chip: `idf_esp32s3_battery` and `idf_esp32_battery`

Device builds need the espup `esp` toolchain (`just setup-toolchain`) and `just setup-cargo-config`.
`just doctor` reports both.

### Hardware tests
`just run <example>` builds, flashes and opens the serial monitor.
The run passes when:
- battery mV and percentage are plausible against a multimeter reading
- the power source is classified correctly (battery-only vs USB)
- the serial output shows no panic, watchdog reset or backtrace
- the run is stable for 60 s and reproducible after a re-flash

Before connecting any LiPo, meter its polarity first (see `docs/key-insights.md` § Hardware).
With USB connected and no battery, both boards report a phantom charge; this is expected.
Record "compile-verified only" honestly when no board is attached.

## Dependency Updates

### Workspace `Cargo.toml`
All shared versions live in the root `[workspace.dependencies]`.
`stoker` is referenced by path plus `version` because both crates are published.

### Pure dependencies (caret ranges)
- `anyhow` and `log` use caret ranges and are refreshed by `just update`.

### ESP-IDF stack
- `esp-idf-hal` and `embuild` move together with the `esp-idf-sys` that `esp-idf-hal` pulls in.
  Each `esp-idf-hal` minor requires a matching `esp-idf-sys` minor and an `embuild` floor.
- `esp-idf-sys` is a `links` crate, so only one version can exist in a downstream graph.
  Lagging the siblings' `esp-idf-hal` minor makes `rustyfarian-power` impossible to combine with them in one app.
  Treat an ESP-IDF wave in the siblings as a trigger for a `rustyfarian-esp-idf-power` minor release.
- `esp-idf-hal` types appear in the public API (constructors take pins and ADC peripherals), so an `esp-idf-hal` minor bump is a breaking change for this crate.
- `ESP_IDF_VERSION` (`.cargo/config.toml.dist`, currently `v5.3.3`) is a separate decision from the crate pins.
  Moving it costs an IDF download and `sdkconfig` re-validation.

### MSRV policy
- The workspace floor (`[workspace.package] rust-version`) is `stoker`'s MSRV and stays as low as the pure core allows.
- `rustyfarian-esp-idf-power` declares its own `rust-version` when the family policy or the ESP stack demands a higher one, matching the siblings' ESP-IDF tier crates.

### Security scanning
- `just deny` runs `cargo deny check` for advisories, licenses, bans and sources.
- `just audit` runs `cargo audit` against the RustSec database; it generates `Cargo.lock` if absent.
- `deny.toml` lists ignored advisories with a rationale and a review-by date.
- Advisories mostly arrive through the `embuild` build-dependency chain and `anyhow`; `just update` usually resolves them.

## CI/CD
GitHub Actions workflows live in `.github/workflows/`:
- `audit.yml` runs the RustSec check on push, PR and a weekly schedule.
- `clippy.yml` and `fmt.yml` are the lint and format gates.
- `rust.yml` runs deny, check and test.

All four call `just` recipes and set `RUSTUP_TOOLCHAIN: stable`.
Keep action versions in step across the four files.
An action's Node runtime is only knowable from `runs.using` in its `action.yml` at the tag in use.
`gh run list` needs `gh auth login`; without it, CI status is NOT VERIFIED.
Dependabot (`.github/dependabot.yml`) watches only the `rust-toolchain` ecosystem.

## Documentation Freshness
Review during quarterly cycles:
- `docs/ROADMAP.md` still reflects priorities and current dependency versions.
- `CHANGELOG.md` `## [Unreleased]` matches the branch state.
- `AGENTS.md` and `README.md` cover MSRV, the crate table and feature flags.
- `docs/key-insights.md` and `docs/project-lore.md` cover CI action versions, MSRV and build conventions; re-date re-confirmed entries.

## Scheduled Maintenance Cadence

### Monthly
- [ ] `just verify` passes.
- [ ] `just audit` and `just deny` are clean apart from documented `deny.toml` ignores.
- [ ] `just update` for in-range transitives; re-run the gates.
- [ ] CI run status (when `gh` is authenticated).

### Quarterly
- [ ] Everything in the monthly checklist.
- [ ] Audit every `[workspace.dependencies]` entry against crates.io.
- [ ] Compare the ESP-IDF pins with `rustyfarian-network`, `-ws2812` and `-peripherals`; the siblings should sit on the same `esp-idf-hal` minor.
- [ ] Check GitHub Actions tags for runtime deprecations.
- [ ] Run the device-target compile checks and one example per chip.
- [ ] Hardware retest on at least one attached board.
- [ ] Re-evaluate `deny.toml` ignores and review `docs/project-lore.md` for accuracy.
- [ ] Decide whether the cycle warrants a crates.io release (see `release-plan.md`).

## Maintenance Protocol
Each cycle produces three files in `audit/` (git-ignored internal logs):
1. `YYYY-MM-DD-<cadence>-audit.md`, a read-only assessment
2. `YYYY-MM-DD-<cadence>-plan.md`, an executable plan derived from the audit
3. `YYYY-MM-DD-<cadence>-maintenance.md`, recording what was actually applied, with outcomes

Behavioural changes from a cycle belong in `CHANGELOG.md` `## [Unreleased]`.
Deferred concerns go in `docs/ROADMAP.md`, and non-obvious technical insights in `docs/project-lore.md`.
Commits are the maintainer's; the cycle leaves a verified working tree and a suggested commit split.
