# Release Plan — 0.1.0

> **Scope of this document.** This is the release artifact for the **0.1.0** publication — a one-off record *and* the reusable template for future releases.
> Instructional steps use placeholders (`<release-date>`, `<version>`); concrete values belong only in the dated `release/<date>-vX.Y.Z-*.md` artifacts.
> For a later release, copy this file forward, bump the version, and re-genericize any values that crept in.
> (The sibling `rustyfarian-network` keeps a single version-specific `release-plan.md`; `rustyfarian-ws2812` keeps a generic plan plus dated `release/` records — this file follows the former with placeholders to slow staleness.)

First publication of `rustyfarian-power` to crates.io.

This release splits the single `battery-monitor` crate into two publishable crates, one per tier, mirroring the sibling repos `rustyfarian-network` (`juggler` + `rustyfarian-esp-idf-network`) and `rustyfarian-ws2812`.
Both crates are released together at version 0.1.0 with coordinated lockstep versioning.

## Crates

- **`stoker`** — the pure, host-buildable core: battery voltage curves, percentage interpolation, power-source detection, sleep validation, wake-cause mapping, charging state, and the `Noop*` mocks. No ESP-IDF dependency; fully testable on the host. (Funfair theme, joining `bunting` / `pennant` / `ferriswheel` / `juggler` — the stoker keeps the fair's engine running.)
- **`rustyfarian-esp-idf-power`** — the ESP-IDF (std) tier: `EspAdcBatteryMonitor`, `EspSleepManager`, `EspWakeCauseSource`, `EspChargingMonitor`, plus the hardware examples. Depends on `stoker` and re-exports its public types at the crate root, so downstream firmware keeps a single `use rustyfarian_esp_idf_power::*` import surface.

`rustyfarian-power` does **not** depend on `rustyfarian-ws2812` or `rustyfarian-network`.
Downstream applications (e.g. `rustyfarian-rgb-clock`) consume all three repos side by side; there is no cross-repo publish ordering.

## Versioning

- **Scheme:** SemVer (pre-1.0 — minor bumps signal breaking changes).
- **Lockstep release:** both crates (`stoker`, `rustyfarian-esp-idf-power`) move together at version 0.1.0.
- **Single source of truth:** `[workspace.package].version` in the root `Cargo.toml` (both member crates inherit via `version.workspace = true`).
- **Independent of siblings:** the family does not lockstep across repos (`rustyfarian-ws2812` is at 0.5.x, `rustyfarian-network` at 0.4.0); 0.1.0 is this repo's honest first publication.
- **Pre-1.0 stability:** breaking API changes are acceptable; semver communicates intent, not a stability promise.

## Phase 0 — Prerequisites (split + API hardening)

This is the first publication, and the repo currently ships a single `battery-monitor` crate.
The following must be complete and merged **before** the dry-run gate can pass.
Track this work in its own approved task (`docs/features/crate-split-power-v1.md`).

### 0.1 Split `battery-monitor` into two crates

- [ ] Create `crates/stoker/` — move `config.rs`, `sleep.rs`, `charging.rs`, and the pure subset of `lib.rs` (traits + `Noop*` mocks). `stoker` keeps a **cfg-only** `build.rs` that emits `cfg(esp32)`/`cfg(esp32s3)` from `TARGET` (plus `rustc-check-cfg`) so `sleep.rs`'s `#[cfg(esp32)]` path resolves — a build script's cfgs do **not** propagate from the ESP-IDF crate to its `stoker` dependency. No `embuild`/linker step in `stoker`'s build script; that lives only in the ESP-IDF crate.
- [ ] Create `crates/rustyfarian-esp-idf-power/` — move `esp_adc.rs`, `esp_sleep.rs`, `esp_charging.rs`, `build.rs`, and all three `idf_*` examples.
- [ ] Promote `validate_gpio_level_source` and `validate_wake_sources` in `sleep.rs` from `pub(crate)` to `pub` (the only visibility change the split forces).
- [ ] Rewrite `crate::…` paths in the three `esp_*.rs` files and the examples to `stoker::…`.
- [ ] Re-export `stoker`'s public types from `rustyfarian-esp-idf-power`'s `lib.rs` root (`pub use stoker::{…};` + the `Esp*` types).
- [ ] Drop the `esp-idf` feature — the crate boundary replaces it; `esp-idf-hal` becomes an unconditional dependency (matches `rustyfarian-esp-idf-network`, which has `default = []` and non-optional `esp-idf-*` deps).
- [ ] Move the device-side `rust,ignore` doc snippet and the `examples/…` GitHub URL out of `stoker`'s `lib.rs` doc into the esp-idf crate; rewrite `battery_monitor::…` doctest paths to `stoker::…`.
- [ ] Update workspace `members` to `crates/stoker` + `crates/rustyfarian-esp-idf-power`; remove the `battery-monitor` directory.
- [ ] Declare the internal dep in `[workspace.dependencies]`: `stoker = { path = "crates/stoker", version = "0.1" }`; the esp-idf crate uses `stoker = { workspace = true }` (path for dev, version at publish).

### 0.2 API hardening (decided: apply now)

- [ ] Add `#[non_exhaustive]` to `PowerSource` (and the other public enums) — `Solar` is a planned variant; this avoids a 0.2.0 break when it lands.
- [ ] **Deferred to 0.2.0 (flag only, not a 0.1.0 blocker):** `SleepManager::sleep` returns `anyhow::Result<()>`, which keeps `stoker` `std`-bound. There is no bare-metal consumer of `stoker` today (power has no esp-hal tier), so `stoker` ships `std` for 0.1.0. A future `no_std` `stoker` swaps `anyhow` for a domain error enum behind an `std` feature (the `juggler` pattern); that is a pre-1.0 minor break.

### 0.3 Per-crate metadata and packaging assets

- [ ] Adopt `[workspace.package].version = "0.1.0"`; both crates set `version.workspace = true`.
- [ ] `crates/stoker/Cargo.toml`: `description`, `keywords`, `categories = ["embedded"]`, `readme = "README.md"`. Do **not** add the `no-std` category at 0.1.0 — `stoker` is `std`-bound this release (see 0.2 below); add `no-std` only when the `no_std` refactor lands in 0.2.0, so the metadata stays honest.
- [ ] `crates/rustyfarian-esp-idf-power/Cargo.toml`: `description`, `keywords` (esp32, esp-idf, battery, power), `categories = ["embedded", "hardware-support"]`, `readme = "README.md"`, `[lib] name = "rustyfarian_esp_idf_power"`, and `[package.metadata.docs.rs] default-target = "riscv32imc-esp-espidf"`.
- [ ] Create `crates/stoker/README.md` and `crates/rustyfarian-esp-idf-power/README.md`.
- [ ] Copy `LICENSE-MIT` and `LICENSE-APACHE` into each crate directory (`cargo publish` includes them in the tarball; `release-validate.sh` step [3/5] checks for them).
- [ ] Update the root `README.md` crate-structure table (one crate → two) and document the two known hardware-tier limitations (see below).

### 0.4 Known hardware-tier limitations to document (not blockers)

- `EspChargingMonitor::new()` bounds `STAT: InputPin + OutputPin`; an input-only GPIO (e.g. Feather V2 GPIO34) cannot satisfy `OutputPin`. Note in the esp-idf crate README.
- `idf_esp32_battery.rs` bypasses `EspSleepManager` and calls `esp_idf_hal::sys` directly for timer sleep (uncertainty about `esp_sleep_config_gpio_isolate` on `xtensa-esp32-espidf`). Note as a latent item.
- `stoker`'s `validate_gpio_level_source` always exercises the ESP32-S3 valid-pin path when built standalone; the `#[cfg(esp32)]` branch only fires as a transitive dep under `rustyfarian-esp-idf-power`. Host tests cannot cover the ESP32 branch.

## Branch and Tag Convention

- **Release branch:** `prepare-crates-publishing` (branched from `main`, code review → fast-forward merge to `main`).
- **Tag format:** `v0.1.0` (annotated, on the release commit).
- **Tagging:** manual step after the `just release-publish-*` recipes succeed for both crates.

## Pre-flight Checklist

Before any publication attempt:

- [ ] Phase 0 complete and merged (split, metadata, packaging assets).
- [ ] Working tree clean on `prepare-crates-publishing` (untracked `review-queue/` and `tmp/` are OK).
- [ ] `just fmt` clean.
- [ ] `just verify` passes (`fmt-check` + `cargo check` + `cargo clippy` + host `cargo test`).
- [ ] `just check-all` passes (ESP32-S3 cross-compile, requires espup).
- [ ] `just check-esp32` passes (Adafruit Feather V2 / `xtensa-esp32-espidf`).
- [ ] At least one hardware example builds per chip via `just build-example <name>` (sanity check, not exhaustive):
  - ESP32-S3: `just build-example idf_esp32s3_battery`
  - ESP32 (Feather V2): `just build-example idf_esp32_battery`
- [ ] `just deny` passes — this is the **blocking** security gate (it honours `deny.toml` licenses, advisories, and bans). `cargo audit` is run too but is **advisory only** (a NOTE, never a hard fail); `release-validate.sh` enforces this split.
- [ ] `CHANGELOG.md` `[0.1.0]` section is cut with entries for this release (see Changelog Update below).
- [ ] Per-crate `README.md` files exist and are accurate.
- [ ] `Cargo.toml` metadata complete for both crates (`description`, `keywords`, `categories`, `readme`, `version.workspace = true`).

## Dry-run Checklist

Before publishing for real, run the one-command pre-flight (it runs, in order: a clean-tracked-tree guard, version lockstep, `just verify`, package-content checks, the `stoker` dry-run, the blocking `cargo deny` gate, and an advisory `cargo audit`):

```shell
just release-publish-validate
```

What it validates and a fundamental ordering constraint:

- **`stoker`** gets a full `cargo publish --dry-run` (it is host-buildable): this verify-builds the crate, packages the tarball (confirming README + dual licenses + all source files are included), and exercises the metadata — with no actual upload.
- **`rustyfarian-esp-idf-power`** gets `cargo package --list` only at this stage. A full `cargo publish --dry-run` for it resolves `stoker ^0.1` **against the crates.io index** (the published manifest drops the local `path`), so it fails with `no matching package named 'stoker' found` until stoker is actually published. This is the same staged constraint the sibling repos handle. Once stoker is live, its real cross-target dry-run runs via `just release-dry-run-idf` (Stage 2 below) before the actual publish.

`rustyfarian-esp-idf-power` is **verify-built against its real cross-compilation target** (`xtensa-esp32s3-espidf`), not the host and **not** `--no-verify`: the default `esp` toolchain compiles `esp-idf-sys` where it actually builds, rather than skipping the verify step.

**Expected outcome:** stoker's dry-run succeeds; rustyfarian-esp-idf-power packages cleanly with README + `LICENSE-MIT` + `LICENSE-APACHE` included.
If stoker's dry-run fails, fix the issue (missing `version`, forbidden path-dep, excluded file) and re-run before publishing.

## Publication Order and Rationale

All publishing is driven through `just` recipes (never raw `cargo publish`), mirroring the sibling convention.
The publish recipes carry a `[confirm]` prompt.
The two crates **must** be published in this staged order.

### Stage 1 — Publish `stoker` first

```shell
just release-publish-stoker
```

(Recipe: `cargo publish -p stoker --target {{host_target}} --all-features`.)
Rationale: `stoker` has no internal crate dependencies; it is self-contained and host-buildable.
Once published and indexed, `rustyfarian-esp-idf-power` can resolve `stoker ^0.1` from crates.io.

Expected time on crates.io: ~2–5 minutes after the command succeeds.

### Stage 2 — Dry-run the dependent crate (now that stoker is live)

This resolves `stoker ^0.1` from the crates.io index and verify-builds against the real cross-target, so it only works after Stage 1 is indexed:

```shell
just release-dry-run-idf
```

If it fails on something other than transient indexing, fix before Stage 3.

### Stage 3 — Publish the dependent crate

```shell
just release-publish-idf
```

(Recipe: `CARGO_TARGET_DIR=… cargo publish -p rustyfarian-esp-idf-power --target xtensa-esp32s3-espidf`.)
It depends on `stoker = "0.1"` (published in Stage 1) and is verify-built against `xtensa-esp32s3-espidf`, not the host: `esp-idf-svc` / `esp-idf-hal` are always-on deps and the crate ships a `build.rs`, so a host verify-build would fail (`esp-idf-sys` rejects the host triple).
Publishing against the Xtensa target runs the verify-build under the `esp` toolchain where it compiles.
(This is why the host target fails — the fix is the correct target, **not** `--no-verify`.)

Expected time on crates.io: ~2–5 minutes after the command succeeds.

**Why not parallel?** crates.io requires all transitive dependencies to be publicly available before a crate can reference them.
Publishing in dependency order (pure tier first, then ESP-IDF tier) ensures each crate resolves cleanly at publish time.

## Changelog Update

Move the `[Unreleased]` section to a dated `[0.1.0]` section in the release commit, **before** running `release-validate.sh` and tagging.
This release also adopts SemVer — update the note at the top of `CHANGELOG.md` (it currently says semver is not yet adopted).

**Before:**

```markdown
## [Unreleased]

### Added
- ...
```

**After:**

```markdown
## [0.1.0] - <release-date>

### Added
- ...

### Changed
- Split the single `battery-monitor` crate into `stoker` (pure core) and `rustyfarian-esp-idf-power` (ESP-IDF tier). Every import path changes: `battery_monitor::X` → `stoker::X` (pure types) or `rustyfarian_esp_idf_power::X` (hardware types). First crates.io publication.

## [Unreleased]
```

Use the actual publication date.
If the date slips, update the `## [0.1.0]` date line before tagging.

## Git Tag and Push

After both crates are published (Stages 1 and 3) and crates.io confirms availability:

```shell
git tag -a v0.1.0 -m "v0.1.0: First publication to crates.io — stoker + rustyfarian-esp-idf-power"
git push origin v0.1.0
```

Then fast-forward `prepare-crates-publishing` to `main`:

```shell
git checkout main
git pull origin main
git merge --ff-only prepare-crates-publishing
git push origin main
```

## GitHub Release

Create a release page at `https://github.com/datenkollektiv/rustyfarian-power/releases/new`:

- **Tag:** `v0.1.0`
- **Title:** `v0.1.0 — First Publication to Crates.io`
- **Body:** keep it short — a one-line summary, the two crates, and the breaking note. Do **not** paste the full `## [0.1.0]` CHANGELOG section. Template:

```markdown
First publication to crates.io. Splits the battery-monitor crate into two publishable crates, one per tier.

## Crates
- **`stoker`** — pure, host-buildable battery / sleep / charging core (no ESP-IDF dependency)
- **`rustyfarian-esp-idf-power`** — ESP-IDF (std) drivers for ESP32-S3 (Heltec V3) and ESP32 (Feather V2)

## Breaking
Import paths change: `battery_monitor::X` → `stoker::X` (pure types) or `rustyfarian_esp_idf_power::X` (hardware types). The esp-idf crate re-exports the pure types at its root, so `use rustyfarian_esp_idf_power::*` covers both.
```

- **Pre-release:** uncheck (stable release).
- **Attachments:** none (library crates, no binary artifacts).

## Post-Publication Verification

Once both crates are on crates.io:

- [ ] Verify `stoker 0.1.0`: https://crates.io/crates/stoker
- [ ] Verify `rustyfarian-esp-idf-power 0.1.0`: https://crates.io/crates/rustyfarian-esp-idf-power
- [ ] docs.rs build expectations:
  - `stoker` docs should build (pure crate, any platform).
  - `rustyfarian-esp-idf-power` docs will most likely **fail to build on docs.rs**: `esp-idf-sys` requires network access and the ESP-IDF C toolchain, which the docs.rs sandbox does not provide. `[package.metadata.docs.rs] default-target = "riscv32imc-esp-espidf"` is set as a best effort (the RISC-V ESP target has docs.rs support; the Xtensa targets do not). Treat a failed build as expected, not a regression — the README on the crates.io page carries the primary documentation. Do **not** set `default-target` to an Xtensa triple (docs.rs rejects it and renders no docs at all).
- [ ] Spot-check an external build that depends on the published crates via `Cargo.toml` version (not path deps) to confirm resolution works.

## Credentials and Registry Authentication

- **crates.io token:** required; obtain from https://crates.io/settings/tokens.
- **Access scope:** both crate names are **new** on crates.io (verified available at planning time — **re-check both names immediately before publishing**, since unclaimed names can be taken between planning and release), so the token must include the **"publish new crates"** scope and must not be allowlisted to other crate names. Token scopes are visible only in the crates.io web UI, not via the API.

**Authentication method: `CARGO_REGISTRY_TOKEN` environment variable.**

`cargo publish` reads `CARGO_REGISTRY_TOKEN` automatically — no `cargo login` and no `~/.cargo/credentials.toml` required.
This repo has no `.envrc` yet; create one (loaded by direnv) mirroring `rustyfarian-network`, or export the variable in the publishing shell:

```shell
export CARGO_REGISTRY_TOKEN="<crates.io token>"
```

Verify the token is present and valid before publishing (does not print the secret):

```shell
test -n "$CARGO_REGISTRY_TOKEN" && echo "token set" || echo "token NOT set"
curl -s -H "Authorization: $CARGO_REGISTRY_TOKEN" https://crates.io/api/v1/me | python3 -m json.tool
```

A JSON body containing your `user` confirms the token is valid (the "publish new crates" scope still needs a one-time visual check in the web UI).
If you prefer `cargo login`, run it once instead; do not use both methods at once.
Never commit the real token to git.

## Rollback Procedure

If a crate must be yanked or the release retracted after publication:

1. **Yank (remove from dependency resolution, keep history):**

   ```shell
   cargo yank --version 0.1.0 <crate>
   ```

   Yank in reverse dependency order (`rustyfarian-esp-idf-power` first, then `stoker`).
   Requires the same token/credentials that published it.

2. **Or delete the release on GitHub** (if not yet heavily used):

   ```shell
   git push --delete origin v0.1.0
   git tag -d v0.1.0
   ```

   Then delete the release page in the GitHub web UI.

**Note:** crates.io versions cannot be truly deleted — only yanked.
Projects with explicit `= 0.1.0` pins fail to resolve after a yank; `^0.1` consumers skip to the next available version.

## Rollback Decision Tree

- **Critical bug <1 hour after publish:** yank the version and re-publish as 0.1.1.
- **Dependency-resolution issue:** yank, fix the root cause, bump to 0.1.1, re-publish.
- **The split itself is wrong:** too late to patch; 0.2.0 (minor, pre-1.0) is the next release, with a migration note in release notes.

## Post-Release Follow-ups

- [ ] Update the root `README.md` to reference the published crates (replace any git-dep examples with crates.io version constraints).
- [ ] Update `docs/ROADMAP.md` to reflect that publication is complete.
- [ ] Create `release/<release-date>-v0.1.0-record.md` documenting: date/time of publication, which crates were published in what order, any dry-run issues and resolutions, and the post-publication verification results. (See the sibling `rustyfarian-ws2812/release/` artifacts for the format; keep a matching `-preflight.md` if useful.)
- [ ] Announce the release on project channels (if applicable).

## Troubleshooting

### Error: `failed to select a version for the requirement 'stoker = "^0.1"'`

**Cause:** `rustyfarian-esp-idf-power` is being published before `stoker` reaches crates.io.
**Fix:** ensure `stoker` is published successfully (check crates.io) before Stage 3.
Wait 2–5 minutes between publishes to allow indexing.

### Error: `failed to publish: metadata.description is missing`

**Cause:** a `[package]` field is missing in `Cargo.toml`.
**Fix:** add the missing field (typically `description`, `keywords`, or `categories`) and re-run the dry-run.

### The esp-idf publish/dry-run fails to build `esp-idf-sys` on the host

**Cause:** the verify-build was attempted on the host target instead of the Xtensa target.
**Fix:** the recipes pass `--target xtensa-esp32s3-espidf` and rely on the `esp` toolchain (espup). Run `just setup-toolchain` first; `rustup show` should list the `esp` toolchain. Do **not** resort to `--no-verify`.

### `cargo +esp` vs `cargo`

This repo pins `channel = "esp"` in `rust-toolchain.toml`, so plain `cargo` already uses the Espressif Xtensa fork.
Unlike `rustyfarian-network`'s RISC-V recipes, the power recipes do not need `cargo +esp`.

## Resources

- [Cargo Book — Publishing](https://doc.rust-lang.org/cargo/reference/publishing.html)
- Sibling reference: `rustyfarian-network/release-plan.md`, `rustyfarian-ws2812/release-plan.md`, and `rustyfarian-ws2812/release/` artifacts.
