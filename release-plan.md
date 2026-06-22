# Release Plan

Reusable steps to cut a `vX.Y.Z` release of the two `rustyfarian-power` crates to crates.io.
Every release gets a `release/<date>-vX.Y.Z-record.md` (timeline, issues, verification) — the mandatory per-release audit record, kept locally (`release/` is gitignored), not published.

## Crates & versioning

- **`stoker`** — pure, host-buildable core. Published **first** (no internal deps).
- **`rustyfarian-esp-idf-power`** — ESP-IDF tier; depends on `stoker = "X.Y"`, re-exports its surface. Published **second**.
- SemVer, pre-1.0 (minor bumps may be breaking). Both crates move in lockstep via `[workspace.package].version`; members inherit with `version.workspace = true`.
- This repo doesn't depend on `rustyfarian-ws2812`/`-network`; no cross-repo publish ordering.

## Critical invariants

Non-negotiables — every release must satisfy these, whatever the recipes happen to do:

- **`stoker` is published and indexed before the esp crate** — crates.io can't resolve `stoker ^X.Y` until it's live (Stage 1 before Stages 2–3).
- **The esp crate verify-builds on `xtensa-esp32s3-espidf`** — never the host, never `--no-verify`; `esp-idf-sys` only compiles on its real target.
- **Both crates move in lockstep** at the same `[workspace.package].version`.
- **Each published crate ships `README.md` + `LICENSE-MIT` + `LICENSE-APACHE`** in its tarball.
- **The release branch carries the final `CHANGELOG.md` cut** (`[Unreleased]` → `[X.Y.Z]`) before publishing and tagging.

## 1. Prepare

- Bump `[workspace.package].version` to `X.Y.Z`.
- Move `CHANGELOG.md` `[Unreleased]` → `[X.Y.Z] - <date>` (leave an empty `[Unreleased]`).
- Commit on a release branch; working tree clean (untracked `review-queue/`, `tmp/`, `.envrc` are fine).

## 2. Pre-flight (non-destructive)

```shell
just release-publish-validate
```
Runs, in order: clean-tracked-tree guard, version lockstep, `just verify`, package contents (README + dual licenses per crate), `stoker` dry-run, blocking `cargo deny`, advisory `cargo audit`.
Also recommended: `just check-all` (ESP32-S3) + `just check-esp32` (Feather V2) + one `just build-example <name>` per chip.

The esp crate gets only `cargo package --list` here — its full dry-run needs `stoker` live first (Stage 2).

## 3. Publish — staged, order matters

Token + esp env must be loaded (`.envrc` via direnv; see Build environment). Always use the `just` recipes, never raw `cargo publish`. The publish recipes carry a `[confirm]` prompt.

```shell
just release-publish-stoker     # Stage 1 — then wait ~2-5 min for crates.io to index
just release-dry-run-idf        # Stage 2 — resolves stoker ^X.Y from the index, verify-builds on xtensa-esp32s3-espidf
just release-publish-idf        # Stage 3 — publishes the esp crate
```
Why staged: crates.io requires `stoker` to be live before the esp crate can reference it. The esp crate verify-builds against its real Xtensa target (the `esp` toolchain), **never** `--no-verify`.

## 4. Tag & GitHub release

```shell
git checkout main && git merge --ff-only <release-branch>
git tag -a vX.Y.Z -m "vX.Y.Z" && git push origin main vX.Y.Z
gh release create vX.Y.Z --title "vX.Y.Z" --notes "<short notes>"
```
Keep notes short — the two crates + any breaking changes. Don't paste the whole changelog.

## 5. Post-publication

- Confirm both crates are live and not yanked (crates.io pages / `curl .../api/v1/crates/<name>`).
- docs.rs: `stoker` builds; **`rustyfarian-esp-idf-power` is expected to fail** (esp-idf-sys needs the ESP-IDF C toolchain + network, absent in the docs.rs sandbox) — not a regression; the README is the primary docs.
- Write `release/<date>-vX.Y.Z-record.md` (timeline, issues, verification).
- Update root `README.md` if it pins crate versions.

## Credentials

`cargo publish` reads `CARGO_REGISTRY_TOKEN` from the environment (loaded via `.envrc`/direnv). The token's scope must permit publishing both crates (a **new** crate name needs the "publish new crates" scope). Never commit the token.

## Build environment

`.envrc` (direnv, gitignored) supplies the build env: `LIBCLANG_PATH` (esp toolchain — no need to `source ~/export-esp.sh`), `RUSTC_WRAPPER=sccache` (global compiler cache, main disk), and `RAMDISK_SIZE_GB`.

**RAM disk sizing matters for publishing:** `cargo publish` runs a fresh cold esp-idf verify-build (~3 GB) in a `package/` dir. The default 6 GB RAM disk, shared across projects, can overflow (`No space left on device`). Keep `RAMDISK_SIZE_GB` ≥ 12, and if it still OOMs, `just ramdisk detach && just ramdisk attach` (resizes) or free space. The failure is pre-upload, so a retry is always clean.

## Rollback

`cargo yank --version X.Y.Z <crate>` in **reverse** dependency order (`rustyfarian-esp-idf-power` first, then `stoker`). crates.io versions can't be deleted, only yanked; `^` consumers skip a yanked version, `=` pins break. To fix: yank, correct the cause, bump the patch, re-publish.

## Troubleshooting

- **`failed to select a version for 'stoker = "^X.Y"'`** — the esp crate was published before `stoker` indexed. Wait 2–5 min, retry Stage 3.
- **esp-idf verify-build fails building `esp-idf-sys` on the host** — it must target `xtensa-esp32s3-espidf` via the `esp` toolchain (`just setup-toolchain`); the recipes already do this. Never use `--no-verify`.
- **`cargo` vs `cargo +esp`** — `rust-toolchain.toml` pins `channel = "esp"`, so plain `cargo` uses the Xtensa fork. No `+esp` needed (unlike the RISC-V sibling repos).

## Resources

- [Cargo Book — Publishing](https://doc.rust-lang.org/cargo/reference/publishing.html)
- Siblings: `rustyfarian-network/release-plan.md`, `rustyfarian-ws2812/release-plan.md` + its `release/` records.
