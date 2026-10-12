# Rustyfarian Power — development tasks
#
# The workspace defaults to the xtensa-esp32s3-espidf target via
# .cargo/config.toml, so host-side recipes pass --target explicitly to
# override it and disable the esp-idf feature.
#
# ESP-IDF builds are isolated to target/idf (vs. host/IDE builds in target/ide).
# Bare-metal (esp-hal) builds are isolated to target/hal.
# An optional macOS RAM disk at /Volumes/RustBuilds accelerates incremental builds.
#
# Run `just setup-toolchain` and `just setup-cargo-config` for first-time setup.

host_target := `scripts/host-target.sh`
# host-side recipes target only the pure `stoker` crate; the ESP-IDF crate
# cannot compile on the host (esp-idf-sys needs the Xtensa/ESP-IDF toolchain).
host_flags  := "-p stoker --target " + host_target
doc_flags   := "-p stoker --target " + host_target + " --no-deps"
esp32s3_target := "xtensa-esp32s3-espidf"
esp32_target   := "xtensa-esp32-espidf"
c3_target      := "riscv32imc-esp-espidf"
hal_crate        := "-p rustyfarian-esp-hal-power"
hal_c3_target    := "riscv32imc-unknown-none-elf"
hal_c6_target    := "riscv32imac-unknown-none-elf"
hal_esp32_target := "xtensa-esp32-none-elf"
hal_s3_target    := "xtensa-esp32s3-none-elf"
# The esp-hal tier is no_std: `-Zbuild-std=core,alloc` on the command line overrides the
# workspace `[unstable] build-std = ["std", "panic_abort"]` in .cargo/config.toml, which
# cannot build for *-none-elf targets (fails in gimli).
hal_std := "-Zbuild-std=core,alloc"

ramdisk := "/Volumes/RustBuilds"
idf_dir := if path_exists(ramdisk + "/targets/idf") == "true" { ramdisk + "/targets/idf/" + file_name(justfile_directory()) } else { "target/idf" }
# bare-metal builds use a separate target dir so they do not thrash the IDF cache
hal_dir := if path_exists(ramdisk + "/targets/hal") == "true" { ramdisk + "/targets/hal/" + file_name(justfile_directory()) } else { "target/hal" }

# list available recipes (default)
_default:
    @just --list

# --- Build & Check --------------------------------------------------------

# check platform-independent code (no ESP toolchain required)
check:
    cargo check {{ host_flags }}

# check all code including ESP-IDF hardware implementations (requires espup)
check-all:
    CARGO_TARGET_DIR="{{ idf_dir }}" cargo check

# check the ESP-IDF power crate for the ESP32 target (Adafruit Feather V2, requires espup)
check-esp32:
    MCU=esp32 CARGO_TARGET_DIR="{{ idf_dir }}" cargo check -p rustyfarian-esp-idf-power --target {{ esp32_target }}

# check the ESP-IDF power crate and its C3 example for the ESP32-C3 target (ESP32-C3-DevKitM-1, requires espup)
check-c3:
    MCU=esp32c3 CARGO_TARGET_DIR="{{ idf_dir }}" cargo check -p rustyfarian-esp-idf-power --target {{ c3_target }} --lib --example idf_c3_deep_sleep_gpio

# verify device-side rustdoc snippets type-check for the ESP32 target (requires espup)
# run this whenever touching rust,ignore doc snippets or esp-idf-gated code
check-docs-esp32:
    MCU=esp32 CARGO_TARGET_DIR="{{ idf_dir }}" cargo check -p rustyfarian-esp-idf-power --target {{ esp32_target }}

# check the esp-hal crate for all four chips (C3, C6, ESP32, S3; esp toolchain, no espup)
check-hal:
    cargo check {{ hal_std }} {{ hal_crate }} --target {{ hal_c3_target }} --target-dir {{ hal_dir }} --no-default-features --features esp32c3,unstable
    cargo check {{ hal_std }} {{ hal_crate }} --target {{ hal_c6_target }} --target-dir {{ hal_dir }} --no-default-features --features esp32c6,unstable
    cargo check {{ hal_std }} {{ hal_crate }} --target {{ hal_esp32_target }} --target-dir {{ hal_dir }} --no-default-features --features esp32,unstable
    cargo check {{ hal_std }} {{ hal_crate }} --target {{ hal_s3_target }} --target-dir {{ hal_dir }} --no-default-features --features esp32s3,unstable

# check the esp-hal crate for the ESP32-C3 only (fast; esp toolchain, no espup)
check-hal-c3:
    cargo check {{ hal_std }} {{ hal_crate }} --target {{ hal_c3_target }} --target-dir {{ hal_dir }} --no-default-features --features esp32c3,unstable

# build platform-independent code (no ESP toolchain required)
build:
    cargo build {{ host_flags }}

# build all code including ESP-IDF hardware implementations (requires espup)
build-all:
    CARGO_TARGET_DIR="{{ idf_dir }}" cargo build

# --- Examples -------------------------------------------------------------

# build a named example without flashing — chip inferred from idf_{chip}_{name} prefix
build-example example:
    CARGO_TARGET_DIR="{{ idf_dir }}" scripts/build-example.sh "{{ example }}" "{{ idf_dir }}"

# build and flash a named example — chip inferred from idf_{chip}_{name} prefix
flash example:
    CARGO_TARGET_DIR="{{ idf_dir }}" scripts/flash.sh "{{ example }}" "{{ idf_dir }}"

# build, flash, and open serial monitor — the human workflow
run example: (flash example)
    espflash monitor

# open serial monitor on the connected device
monitor:
    espflash monitor

# erase the connected device's flash completely (use before reflashing on boot failures)
[confirm]
erase-flash:
    espflash erase-flash

# --- Code Quality ---------------------------------------------------------

# run clippy on platform-independent code
clippy:
    cargo clippy {{ host_flags }} -- -D warnings

# run clippy on all code including ESP-IDF (requires espup)
clippy-all:
    CARGO_TARGET_DIR="{{ idf_dir }}" cargo clippy -- -D warnings

# run clippy on the esp-hal crate for all four chips (C3, C6, ESP32, S3; esp toolchain, no espup)
clippy-hal:
    cargo clippy {{ hal_std }} {{ hal_crate }} --target {{ hal_c3_target }} --target-dir {{ hal_dir }} --no-default-features --features esp32c3,unstable -- -D warnings
    cargo clippy {{ hal_std }} {{ hal_crate }} --target {{ hal_c6_target }} --target-dir {{ hal_dir }} --no-default-features --features esp32c6,unstable -- -D warnings
    cargo clippy {{ hal_std }} {{ hal_crate }} --target {{ hal_esp32_target }} --target-dir {{ hal_dir }} --no-default-features --features esp32,unstable -- -D warnings
    cargo clippy {{ hal_std }} {{ hal_crate }} --target {{ hal_s3_target }} --target-dir {{ hal_dir }} --no-default-features --features esp32s3,unstable -- -D warnings

# run host-side unit tests (no ESP toolchain required)
test:
    cargo test {{ host_flags }}

# run host-side tests with stdout/stderr visible
test-verbose:
    cargo test {{ host_flags }} -- --nocapture

# run a single named test
test-one name:
    cargo test {{ host_flags }} {{ name }}

# format all code
fmt:
    cargo fmt

# check formatting without modifying files
fmt-check:
    cargo fmt -- --check

# --- Documentation --------------------------------------------------------

# build rustdoc for platform-independent code
doc:
    cargo doc {{ doc_flags }}

# build and open docs in browser
doc-open:
    cargo doc {{ doc_flags }} --open

# --- Build Environment ────────────────────────────────────────────────────

# report development tooling status (Rust, esp toolchain, espflash, ESP-IDF, RAM disk)
doctor:
    @scripts/doctor.sh "{{ ramdisk }}" "{{ idf_dir }}"

# manage the RAM disk: just ramdisk attach | detach
ramdisk action:
    @scripts/ramdisk.sh "{{action}}"

# ensure the IDF-built v5.3.3 bootloader is cached for the given chip
ensure-bootloader chip:
    CARGO_TARGET_DIR="{{ idf_dir }}" scripts/ensure-bootloader.sh "{{chip}}" "{{idf_dir}}"

# --- Maintenance ----------------------------------------------------------

# check dependency licenses, advisories, and bans
deny:
    cargo deny check

# audit dependencies for known security advisories (RUSTSEC)
audit:
    [ -f Cargo.lock ] || cargo generate-lockfile
    cargo audit

# update dependencies
update:
    cargo update

# clean only the bare-metal (esp-hal) target dir; keeps the ESP-IDF cache
clean-hal:
    cargo clean --target-dir {{ hal_dir }}

# clean build artifacts (target/ide, target/idf, target/hal, and RAM disk if mounted)
clean:
    cargo clean --target-dir target/ide
    cargo clean --target-dir {{ idf_dir }}
    cargo clean --target-dir {{ hal_dir }}

# --- Composite ------------------------------------------------------------

# full pre-commit verification: format, check, lint, test (modifies files — local use only)
pre-commit: fmt check clippy test

# non-modifying full verification (format check, deny, check, lint, test): fails on any anomaly
verify:
    @cargo fmt -- --check || (printf '\nFormatting issues found — run `just pre-commit` to auto-fix.\n' >&2 && exit 1)
    cargo deny check
    cargo check {{ host_flags }}
    cargo clippy {{ host_flags }} -- -D warnings
    cargo test {{ host_flags }}

# CI-equivalent verification (non-modifying): format check, deny, check, lint, test
ci: fmt-check deny check clippy test

# --- Release --------------------------------------------------------------
#
# Three crates publish in dependency order: stoker (pure, host-buildable) first,
# then the two tiers, rustyfarian-esp-idf-power and rustyfarian-esp-hal-power,
# which resolve stoker ^0.2 from crates.io once it is live. The default
# toolchain is the esp channel, so plain `cargo` drives the Xtensa fork — no
# `cargo +esp` needed.
# See release-plan.md for the full staged sequence, credentials, and rollback.

# show the staged release flow at a glance (full detail in release-plan.md)
release-help:
    @echo "Staged release flow (see release-plan.md for detail):"
    @echo "  0. Phase 0 prerequisites merged: crate split + per-crate README/LICENSE + metadata"
    @echo "  1. just release-publish-validate   # preflight gate (no upload)"
    @echo "  2. just release-publish-stoker     # Stage 1: publish pure crate, then wait ~2-5 min to index"
    @echo "  3. just release-dry-run-idf        # Stage 2: resolves stoker ^0.2 from crates.io"
    @echo "  4. just release-publish-idf        # Stage 3: publish ESP-IDF crate (xtensa-esp32s3-espidf)"
    @echo "  5. just release-dry-run-hal        # Stage 4: esp-hal crate (riscv32imc-unknown-none-elf, -Zbuild-std=core,alloc)"
    @echo "  6. just release-publish-hal        # Stage 5: publish esp-hal crate"
    @echo "  7. git tag -a v<version> && git push --tags, then cut the GitHub release"

# pre-flight release validation (clean-tree guard, version lockstep, verify, package contents, stoker dry-run, deny, audit) — see release-plan.md
release-publish-validate:
    scripts/release-validate.sh

# dry-run package stoker against the host target (pure crate; no upload)
release-dry-run-stoker:
    cargo publish --dry-run -p stoker --target {{ host_target }} --all-features

# dry-run package rustyfarian-esp-idf-power against the ESP32-S3 target (no upload; requires espup)
# NOTE: only succeeds AFTER stoker is published to crates.io (resolves stoker ^0.2 from the index)
release-dry-run-idf:
    CARGO_TARGET_DIR="{{ idf_dir }}" cargo publish --dry-run -p rustyfarian-esp-idf-power --target {{ esp32s3_target }}

# dry-run package rustyfarian-esp-hal-power against the ESP32-C3 target (no upload; esp toolchain, no espup)
# NOTE: only succeeds AFTER stoker is published to crates.io (resolves stoker ^0.2 from the index)
release-dry-run-hal:
    cargo publish --dry-run {{ hal_std }} -p rustyfarian-esp-hal-power --target {{ hal_c3_target }} --target-dir {{ hal_dir }}

# Stage 1 — publish stoker (pure) to crates.io
[confirm]
release-publish-stoker:
    cargo publish -p stoker --target {{ host_target }} --all-features

# Stage 3 — publish rustyfarian-esp-idf-power to crates.io (requires espup; run after stoker is indexed)
[confirm]
release-publish-idf:
    CARGO_TARGET_DIR="{{ idf_dir }}" cargo publish -p rustyfarian-esp-idf-power --target {{ esp32s3_target }}

# Stage 4 — publish rustyfarian-esp-hal-power to crates.io (esp toolchain; run after stoker is indexed)
[confirm]
release-publish-hal:
    cargo publish {{ hal_std }} -p rustyfarian-esp-hal-power --target {{ hal_c3_target }} --target-dir {{ hal_dir }}

# --- Setup ----------------------------------------------------------------

# copy the cargo config template for first-time setup
setup-cargo-config:
    cp .cargo/config.toml.dist .cargo/config.toml

# install the ESP-IDF toolchain via espup
setup-toolchain:
    espup install
