# Release Plan

Reusable steps to cut a `vX.Y.Z` release of the three `rustyfarian-power` crates to crates.io.
Every release gets a `release/<date>-vX.Y.Z-record.md` (timeline, issues, verification) — the mandatory per-release audit record, kept locally (`release/` is gitignored), not published.

## Crates & versioning

- **`stoker`** — pure, host-buildable core. Published **first** (no internal deps).
- **`rustyfarian-esp-idf-power`** — ESP-IDF tier; depends on `stoker = "X.Y"`, re-exports its surface. Published **second**.
- **`rustyfarian-esp-hal-power`** — bare-metal esp-hal tier (`no_std`); depends on `stoker = "X.Y"` and pins `esp-hal = "=1.2.2"`. Published **third**.
- SemVer, pre-1.0 (minor bumps may be breaking). All three crates move in lockstep via `[workspace.package].version`; members inherit with `version.workspace = true`.
- This repo doesn't depend on `rustyfarian-ws2812`/`-network`; no cross-repo publish ordering.

## Critical invariants

Non-negotiables — every release must satisfy these, whatever the recipes happen to do:

- **`stoker` is published and indexed before the tier crates** — crates.io can't resolve `stoker ^X.Y` until it's live (Stage 1 before Stages 2–5).
- **The ESP-IDF crate verify-builds on `xtensa-esp32s3-espidf`** — never the host, never `--no-verify`; `esp-idf-sys` only compiles on its real target.
- **The esp-hal crate verify-builds on `riscv32imc-unknown-none-elf`** with `-Zbuild-std=core,alloc` and its default `esp32c3` feature — never `--no-verify`.
- **All three crates move in lockstep** at the same `[workspace.package].version`.
- **Each published crate ships `README.md` + `LICENSE-MIT` + `LICENSE-APACHE`** in its tarball.
- **The release branch carries the final `CHANGELOG.md` cut** (`[Unreleased]` → `[X.Y.Z]`) before publishing and tagging.

## 1. Prepare

- Release branch is `main`; merge any pending work branch into it first.
- Bump `[workspace.package].version` to `X.Y.Z`.
- On a minor bump, also bump the `stoker` entry in `[workspace.dependencies]` to `version = "X.Y"`.
- Move `CHANGELOG.md` `[Unreleased]` → `[X.Y.Z] - <date>` (leave an empty `[Unreleased]`).
- Commit on `main`; working tree clean (untracked `review-queue/`, `tmp/`, `.envrc` are fine).

## 2. Pre-flight (non-destructive)

```shell
just release-publish-validate
```
Runs, in order: clean-tracked-tree guard, version lockstep, `just verify`, package contents (README + dual licenses per crate), `stoker` dry-run, blocking `cargo deny`, advisory `cargo audit`.
Also recommended: `just check-all` (ESP32-S3) + `just check-esp32` (Feather V2) + `just check-c3` + `just check-hal` + one `just build-example <name>` per chip.

The tier crates get only `cargo package --list` here — their full dry-runs need `stoker` live first (Stages 2 and 4).

## 3. Publish — staged, order matters

Token + esp env must be loaded (`.envrc` via direnv; see Build environment). Always use the `just` recipes, never raw `cargo publish`. The publish recipes carry a `[confirm]` prompt.

```shell
just release-publish-stoker
just release-dry-run-idf
just release-publish-idf
just release-dry-run-hal
just release-publish-hal
```
- Stage 1 publishes `stoker`; wait ~2–5 min for crates.io to index it.
- Stage 2 resolves `stoker ^X.Y` from the index and verify-builds the ESP-IDF crate on `xtensa-esp32s3-espidf`.
- Stage 3 publishes the ESP-IDF crate.
- Stage 4 resolves `stoker ^X.Y` from the index and verify-builds the esp-hal crate on `riscv32imc-unknown-none-elf`.
- Stage 5 publishes the esp-hal crate.

Why staged: crates.io requires `stoker` to be live before a tier crate can reference it.
Each tier crate verify-builds against its real target (the `esp` toolchain), **never** `--no-verify`.

## 4. Tag & GitHub release

```shell
git tag -a vX.Y.Z -m "vX.Y.Z" && git push origin main vX.Y.Z
gh release create vX.Y.Z --title "vX.Y.Z" --notes "<short notes>"
```
Keep notes short — the three crates + any breaking changes. Don't paste the whole changelog.

## 5. Post-publication

- Confirm all three crates are live and not yanked (crates.io pages / `curl .../api/v1/crates/<name>`).
- docs.rs: `stoker` builds; **`rustyfarian-esp-idf-power` is expected to fail** (esp-idf-sys needs the ESP-IDF C toolchain + network, absent in the docs.rs sandbox) — not a regression; the README is the primary docs.
- docs.rs: `rustyfarian-esp-hal-power` is expected to build (`riscv32imc-unknown-none-elf`, feature `esp32c3`, from its `[package.metadata.docs.rs]`); a failure there needs a look.
- Write `release/<date>-vX.Y.Z-record.md` (timeline, issues, verification).
- Update root `README.md` if it pins crate versions.

## Credentials

`cargo publish` reads `CARGO_REGISTRY_TOKEN` from the environment (loaded via `.envrc`/direnv). The token's scope must permit publishing all three crates (a **new** crate name needs the "publish new crates" scope). Never commit the token.

## Build environment

`.envrc` (direnv, gitignored) supplies the build env: `LIBCLANG_PATH` (esp toolchain — no need to `source ~/export-esp.sh`), `RUSTC_WRAPPER=sccache` (global compiler cache, main disk), and `RAMDISK_SIZE_GB`.

**RAM disk sizing matters for publishing:** `cargo publish` runs a fresh cold esp-idf verify-build (~3 GB) in a `package/` dir. The default 6 GB RAM disk, shared across projects, can overflow (`No space left on device`). Keep `RAMDISK_SIZE_GB` ≥ 12, and if it still OOMs, `just ramdisk detach && just ramdisk attach` (resizes) or free space. The failure is pre-upload, so a retry is always clean.

## Rollback

`cargo yank --version X.Y.Z <crate>` in **reverse** dependency order (`rustyfarian-esp-hal-power` and `rustyfarian-esp-idf-power` first, then `stoker`). crates.io versions can't be deleted, only yanked; `^` consumers skip a yanked version, `=` pins break. To fix: yank, correct the cause, bump the patch, re-publish.

## Troubleshooting

- **`failed to select a version for 'stoker = "^X.Y"'`** — a tier crate was published before `stoker` indexed. Wait 2–5 min, retry Stage 2 or 4.
- **esp-idf verify-build fails building `esp-idf-sys` on the host** — it must target `xtensa-esp32s3-espidf` via the `esp` toolchain (`just setup-toolchain`); the recipes already do this. Never use `--no-verify`.
- **`cargo` vs `cargo +esp`** — `rust-toolchain.toml` pins `channel = "esp"`, so plain `cargo` uses the Xtensa fork. No `+esp` needed (unlike the RISC-V sibling repos).

## Resources

- [Cargo Book — Publishing](https://doc.rust-lang.org/cargo/reference/publishing.html)
- Siblings: `rustyfarian-network/release-plan.md`, `rustyfarian-ws2812/release-plan.md` + its `release/` records.
