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
# Run `just setup-toolchain` for first-time setup.

host_target := `scripts/host-target.sh`
# host-side recipes target only the pure `stoker` crate; the ESP-IDF crate
# cannot compile on the host (esp-idf-sys needs the Xtensa/ESP-IDF toolchain).
host_flags  := "-p stoker --target " + host_target
doc_flags   := "-p stoker --target " + host_target + " --no-deps"
esp32s3_target := "xtensa-esp32s3-espidf"
esp32_target   := "xtensa-esp32-espidf"
c3_target      := "riscv32imc-esp-espidf"
c6_target      := "riscv32imac-esp-espidf"
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

# Guided bench runner: a local rustyfarian-bench checkout if RUSTYFARIAN_BENCH points at one,
# otherwise the pinned release from GitHub.
bench_dir := env("RUSTYFARIAN_BENCH", "")
bench := if bench_dir != "" { 'uv run --project "' + bench_dir + '" rustyfarian-bench' } else { "uvx --from git+https://github.com/datenkollektiv/rustyfarian-bench@v0.5.0 rustyfarian-bench" }

# list available recipes (default)
_default:
    @just --list

# --- Build & Check --------------------------------------------------------

# check platform-independent code (no ESP toolchain required)
check:
    cargo check {{ host_flags }}

# check all code including ESP-IDF hardware implementations (requires espup)
check-all:
    CARGO_TARGET_DIR="{{ idf_dir }}" cargo check -p rustyfarian-esp-idf-power --features critical-section-impl

# check every workspace member against the default target, as the IDE does (requires espup)
check-ide:
    cargo check --workspace

# check the ESP-IDF power crate for the ESP32 target (Adafruit Feather V2, requires espup)
check-esp32:
    MCU=esp32 CARGO_TARGET_DIR="{{ idf_dir }}" cargo check -p rustyfarian-esp-idf-power --features critical-section-impl --target {{ esp32_target }}

# check the ESP-IDF power crate and its C3 examples for the ESP32-C3 target (ESP32-C3-DevKitM-1, requires espup)
check-c3:
    MCU=esp32c3 CARGO_TARGET_DIR="{{ idf_dir }}" cargo check -p rustyfarian-esp-idf-power --features critical-section-impl --target {{ c3_target }} --lib --example idf_c3_deep_sleep_gpio --example idf_c3_rtc_retention

# check the ESP-IDF power crate and its C6 examples for the ESP32-C6 target (ESP32-C6-DevKitC-1, requires espup)
check-c6:
    MCU=esp32c6 CARGO_TARGET_DIR="{{ idf_dir }}" cargo check -p rustyfarian-esp-idf-power --features critical-section-impl --target {{ c6_target }} --lib --example idf_c6_deep_sleep_gpio --example idf_c6_rtc_retention

# verify device-side rustdoc snippets type-check for the ESP32 target (requires espup)
# run this whenever touching rust,ignore doc snippets or esp-idf-gated code
check-docs-esp32:
    MCU=esp32 CARGO_TARGET_DIR="{{ idf_dir }}" cargo check -p rustyfarian-esp-idf-power --features critical-section-impl --target {{ esp32_target }}

# check the esp-hal crate for all four chips (C3, C6, ESP32, S3; esp toolchain, no espup); the first line is the stoker-only surface without `unstable`
check-hal:
    cargo check {{ hal_std }} {{ hal_crate }} --target {{ hal_c3_target }} --target-dir {{ hal_dir }} --no-default-features --features esp32c3
    cargo check {{ hal_std }} {{ hal_crate }} --target {{ hal_c3_target }} --target-dir {{ hal_dir }} --no-default-features --features esp32c3,unstable
    cargo check {{ hal_std }} {{ hal_crate }} --target {{ hal_c6_target }} --target-dir {{ hal_dir }} --no-default-features --features esp32c6,unstable
    cargo check {{ hal_std }} {{ hal_crate }} --target {{ hal_esp32_target }} --target-dir {{ hal_dir }} --no-default-features --features esp32,unstable
    cargo check {{ hal_std }} {{ hal_crate }} --target {{ hal_s3_target }} --target-dir {{ hal_dir }} --no-default-features --features esp32s3,unstable
    cargo check {{ hal_std }} {{ hal_crate }} --target {{ hal_s3_target }} --target-dir {{ hal_dir }} --no-default-features --features esp32s3,unstable,examples --examples
    cargo check {{ hal_std }} {{ hal_crate }} --target {{ hal_esp32_target }} --target-dir {{ hal_dir }} --no-default-features --features esp32,unstable,examples --examples
    cargo check {{ hal_std }} {{ hal_crate }} --target {{ hal_c6_target }} --target-dir {{ hal_dir }} --no-default-features --features esp32c6,unstable,examples --examples
    BENCH_CASE=H1 cargo check {{ hal_std }} {{ hal_crate }} --target {{ hal_c3_target }} --target-dir {{ hal_dir }} --no-default-features --features esp32c3,unstable,examples,bench --examples

# check the esp-hal crate for the ESP32-C3 only (fast; esp toolchain, no espup)
check-hal-c3:
    cargo check {{ hal_std }} {{ hal_crate }} --target {{ hal_c3_target }} --target-dir {{ hal_dir }} --no-default-features --features esp32c3,unstable

# build platform-independent code (no ESP toolchain required)
build:
    cargo build {{ host_flags }}

# build all code including ESP-IDF hardware implementations (requires espup)
build-all:
    CARGO_TARGET_DIR="{{ idf_dir }}" cargo build -p rustyfarian-esp-idf-power --features critical-section-impl

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

# build a named esp-hal example for S3 (no_std bare-metal, release optimization)
build-example-hal-s3 example:
    cargo build {{ hal_std }} {{ hal_crate }} --release --target {{ hal_s3_target }} --target-dir {{ hal_dir }} --no-default-features --features esp32s3,unstable,examples --example {{ example }}

# build and flash a named esp-hal S3 example, then open serial monitor
run-hal-s3 example: (build-example-hal-s3 example)
    espflash flash --monitor --chip esp32s3 {{ hal_dir }}/{{ hal_s3_target }}/release/examples/{{ example }}

# build and flash a named esp-hal S3 example without opening the monitor (bench host scripts attach their own reader)
flash-hal-s3 example: (build-example-hal-s3 example)
    espflash flash --chip esp32s3 {{ hal_dir }}/{{ hal_s3_target }}/release/examples/{{ example }}

# build a named esp-hal example for C3 (no_std bare-metal, release optimization, optional features)
build-example-hal-c3 example features="esp32c3,unstable,examples":
    cargo build {{ hal_std }} {{ hal_crate }} --release --target {{ hal_c3_target }} --target-dir {{ hal_dir }} --no-default-features --features {{ features }} --example {{ example }}

# build and flash a named esp-hal C3 example, then open serial monitor
run-hal-c3 example features="esp32c3,unstable,examples": (build-example-hal-c3 example features)
    espflash flash --monitor --chip esp32c3 {{ hal_dir }}/{{ hal_c3_target }}/release/examples/{{ example }}

# build and flash a named esp-hal C3 example without opening the monitor (bench host scripts attach their own reader)
flash-hal-c3 example features="esp32c3,unstable,examples": (build-example-hal-c3 example features)
    espflash flash --chip esp32c3 {{ hal_dir }}/{{ hal_c3_target }}/release/examples/{{ example }}

# build a named esp-hal example for C6 (no_std bare-metal, release optimization, optional features)
build-example-hal-c6 example features="esp32c6,unstable,examples":
    cargo build {{ hal_std }} {{ hal_crate }} --release --target {{ hal_c6_target }} --target-dir {{ hal_dir }} --no-default-features --features {{ features }} --example {{ example }}

# build and flash a named esp-hal C6 example, then open serial monitor
run-hal-c6 example features="esp32c6,unstable,examples": (build-example-hal-c6 example features)
    espflash flash --monitor --chip esp32c6 {{ hal_dir }}/{{ hal_c6_target }}/release/examples/{{ example }}

# build and flash a named esp-hal C6 example without opening the monitor (bench host scripts attach their own reader)
flash-hal-c6 example features="esp32c6,unstable,examples": (build-example-hal-c6 example features)
    espflash flash --chip esp32c6 {{ hal_dir }}/{{ hal_c6_target }}/release/examples/{{ example }}

# build a named esp-hal example for ESP32 (no_std bare-metal, release optimization, optional features)
build-example-hal-esp32 example features="esp32,unstable,examples":
    cargo build {{ hal_std }} {{ hal_crate }} --release --target {{ hal_esp32_target }} --target-dir {{ hal_dir }} --no-default-features --features {{ features }} --example {{ example }}

# build and flash a named esp-hal ESP32 example, then open serial monitor
run-hal-esp32 example features="esp32,unstable,examples": (build-example-hal-esp32 example features)
    espflash flash --monitor --chip esp32 {{ hal_dir }}/{{ hal_esp32_target }}/release/examples/{{ example }}

# build and flash a named esp-hal ESP32 example without opening the monitor (bench host scripts attach their own reader)
flash-hal-esp32 example features="esp32,unstable,examples": (build-example-hal-esp32 example features)
    espflash flash --chip esp32 {{ hal_dir }}/{{ hal_esp32_target }}/release/examples/{{ example }}

# guided hardware check: `just bench c3 retention`, `just bench hal_c3_rtc_retention`, `just bench c3` (see bench/README.md)
bench target *args:
    {{ bench }} --repo . run "{{ target }}" {{ args }}

# list the bench scenarios
bench-list:
    {{ bench }} --repo . list

# probe the connected boards (chip and MAC per port)
bench-detect:
    {{ bench }} detect

# Bench build step (bench/bench.toml): `idf_*` via build-example, `hal_<chip>_*` via build-example-hal-<chip>; `NAME+VARIANT` sets BENCH_VARIANT
bench-build NAME:
    #!/usr/bin/env bash
    set -euo pipefail
    # the esp-hal Xtensa links need xtensa-esp32-elf-gcc on PATH (no `source ~/export-esp.sh` required)
    # `NAME+VARIANT` builds NAME with BENCH_VARIANT=VARIANT (examples that support variants read it at build time)
    name="{{ NAME }}"
    base="${name%%+*}"
    if [ "$base" != "$name" ]; then export BENCH_VARIANT="${name#*+}"; else unset BENCH_VARIANT; fi
    . scripts/xtensa-toolchain.sh
    case "$base" in hal_esp32_* | hal_s3_*) setup_xtensa_toolchain ;; esac
    case "$base" in
        idf_*)       just build-example "$base" ;;
        hal_c3_*)    just build-example-hal-c3 "$base" ;;
        hal_c6_*)    just build-example-hal-c6 "$base" ;;
        hal_esp32_*) just build-example-hal-esp32 "$base" ;;
        hal_s3_*)    just build-example-hal-s3 "$base" ;;
        *) echo "bench-build: unknown name {{ NAME }} (idf_*, hal_{c3,c6,esp32,s3}_*)" >&2; exit 1 ;;
    esac

# Bench flash step (bench/bench.toml): flash a bench-build artifact to PORT, without a monitor (espflash reads ESPFLASH_PORT)
bench-flash NAME PORT:
    #!/usr/bin/env bash
    set -euo pipefail
    # the esp-hal Xtensa links need xtensa-esp32-elf-gcc on PATH (no `source ~/export-esp.sh` required)
    # `NAME+VARIANT` builds NAME with BENCH_VARIANT=VARIANT (examples that support variants read it at build time)
    name="{{ NAME }}"
    base="${name%%+*}"
    if [ "$base" != "$name" ]; then export BENCH_VARIANT="${name#*+}"; else unset BENCH_VARIANT; fi
    . scripts/xtensa-toolchain.sh
    case "$base" in hal_esp32_* | hal_s3_*) setup_xtensa_toolchain ;; esac
    export ESPFLASH_PORT="{{ PORT }}"
    case "$base" in
        idf_*)       just flash "$base" ;;
        hal_c3_*)    just flash-hal-c3 "$base" ;;
        hal_c6_*)    just flash-hal-c6 "$base" ;;
        hal_esp32_*) just flash-hal-esp32 "$base" ;;
        hal_s3_*)    just flash-hal-s3 "$base" ;;
        *) echo "bench-flash: unknown name {{ NAME }} (idf_*, hal_{c3,c6,esp32,s3}_*)" >&2; exit 1 ;;
    esac

# run the C3 bench host driver (wraps `uv run scripts/c3-bench.py`)
[positional-arguments]
bench-c3 *args:
    uv run scripts/c3-bench.py "$@"

# capture a serial port for N seconds, DTR/RTS untouched, reopening after re-enumeration (wraps `uv run scripts/serial-capture.py`); e.g. just serial-capture /dev/cu.usbserial-210 30
[positional-arguments]
serial-capture port seconds baud="115200":
    uv run scripts/serial-capture.py "$@"

# stream a serial console with timestamps until Ctrl-C, with a banner when the PIR warm-up is over (default 60 s); DTR/RTS held low, reopens a dropped port; e.g. just console /dev/cu.usbserial-0001 60
[positional-arguments]
console port warmup="60" baud="115200":
    uv run scripts/serial-console.py "$@"

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
    CARGO_TARGET_DIR="{{ idf_dir }}" cargo clippy -p stoker -p rustyfarian-esp-idf-power --features rustyfarian-esp-idf-power/critical-section-impl -- -D warnings

# run clippy on the esp-hal crate for all four chips (C3, C6, ESP32, S3; esp toolchain, no espup)
clippy-hal:
    cargo clippy {{ hal_std }} {{ hal_crate }} --target {{ hal_c3_target }} --target-dir {{ hal_dir }} --no-default-features --features esp32c3,unstable -- -D warnings
    cargo clippy {{ hal_std }} {{ hal_crate }} --target {{ hal_c6_target }} --target-dir {{ hal_dir }} --no-default-features --features esp32c6,unstable -- -D warnings
    cargo clippy {{ hal_std }} {{ hal_crate }} --target {{ hal_esp32_target }} --target-dir {{ hal_dir }} --no-default-features --features esp32,unstable -- -D warnings
    cargo clippy {{ hal_std }} {{ hal_crate }} --target {{ hal_s3_target }} --target-dir {{ hal_dir }} --no-default-features --features esp32s3,unstable -- -D warnings
    cargo clippy {{ hal_std }} {{ hal_crate }} --target {{ hal_s3_target }} --target-dir {{ hal_dir }} --no-default-features --features esp32s3,unstable,examples --examples -- -D warnings
    cargo clippy {{ hal_std }} {{ hal_crate }} --target {{ hal_esp32_target }} --target-dir {{ hal_dir }} --no-default-features --features esp32,unstable,examples --examples -- -D warnings
    cargo clippy {{ hal_std }} {{ hal_crate }} --target {{ hal_c6_target }} --target-dir {{ hal_dir }} --no-default-features --features esp32c6,unstable,examples --examples -- -D warnings
    BENCH_CASE=H1 cargo clippy {{ hal_std }} {{ hal_crate }} --target {{ hal_c3_target }} --target-dir {{ hal_dir }} --no-default-features --features esp32c3,unstable,examples,bench --examples -- -D warnings

# run host-side unit tests (no ESP toolchain required)
test:
    cargo test {{ host_flags }}

# run host-side tests with the off-by-default `bench-conformance` feature (shared bench grammar doctest)
test-bench:
    cargo test {{ host_flags }} --features bench-conformance

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

# ensure the IDF-built bootloader is cached for the given chip
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

# clean build artifacts: the whole local target/ (also builds from before the RAM disk was mounted), then the RAM disk dirs if mounted
clean:
    cargo clean --target-dir target
    cargo clean --target-dir {{ idf_dir }}
    cargo clean --target-dir {{ hal_dir }}

# --- CI chip jobs ---------------------------------------------------------
#
# One aggregate recipe per CI job (.github/workflows/rust.yml, hal.yml), so CI and a
# local run execute the same steps. Provider failures are link-time facts (ADR-003
# point 5), so these link the representative firmware per tier and chip, which
# `cargo check` cannot do. The original ESP32 is compile-only: no link builds.

# CI: ESP-IDF tier on the C3: check, then link both C3 examples (requires espup)
ci-c3: check-c3 (build-example "idf_c3_deep_sleep_gpio") (build-example "idf_c3_rtc_retention")

# CI: ESP-IDF tier on the C6: check, then link both C6 examples (requires espup)
ci-c6: check-c6 (build-example "idf_c6_deep_sleep_gpio") (build-example "idf_c6_rtc_retention")

# CI: ESP-IDF tier on the S3: link all three S3 examples (requires espup)
ci-s3: (build-example "idf_s3_rtc_retention") (build-example "idf_s3_deep_sleep_gpio") (build-example "idf_s3_power_rail_gate")

# CI: ESP-IDF tier on the original ESP32, compile-only (no link build, hardware deferred; requires espup)
ci-esp32: check-esp32

# CI: esp-hal tier: check all four chips, then link the C3, C6, ESP32 and S3 examples plus one bench case (esp toolchain, no espup)
ci-hal: check-hal (build-example-hal-c3 "hal_c3_deep_sleep_gpio") (build-example-hal-c3 "hal_c3_rtc_retention") (build-example-hal-s3 "hal_s3_deep_sleep_gpio") (build-example-hal-s3 "hal_s3_power_rail_gate") (build-example-hal-s3 "hal_s3_battery") (build-example-hal-esp32 "hal_esp32_battery") (build-example-hal-esp32 "hal_esp32_deep_sleep_gpio") (build-example-hal-c6 "hal_c6_deep_sleep_gpio")
    BENCH_CASE=H1 just build-example-hal-c3 hal_c3_sleep_bench esp32c3,unstable,examples,bench

# --- Composite ------------------------------------------------------------

# full pre-commit verification: format, check, lint, test (modifies files — local use only)
pre-commit: fmt check clippy test

# non-modifying full verification (format check, deny, check, lint, test): fails on any anomaly
verify:
    @cargo fmt -- --check || (printf '\nFormatting issues found — run `just pre-commit` to auto-fix.\n' >&2 && exit 1)
    cargo deny check
    cargo check {{ host_flags }}
    cargo clippy {{ host_flags }} -- -D warnings
    cargo clippy {{ host_flags }} --features bench-conformance -- -D warnings
    cargo test {{ host_flags }}
    cargo test {{ host_flags }} --features bench-conformance

# CI-equivalent verification (non-modifying): format check, deny, check, lint, test
ci: fmt-check deny check clippy test test-bench

# --- Release --------------------------------------------------------------
#
# Three crates publish in dependency order: stoker (pure, host-buildable) first,
# then the two tiers, rustyfarian-esp-idf-power and rustyfarian-esp-hal-power,
# which resolve stoker ^X.Y from crates.io once it is live. The default
# toolchain is the esp channel, so plain `cargo` drives the Xtensa fork — no
# `cargo +esp` needed.
# See release-plan.md for the full staged sequence, credentials, and rollback.

# show the staged release flow at a glance (full detail in release-plan.md)
release-help:
    @echo "Staged release flow (see release-plan.md for detail):"
    @echo "  1. just release-publish-validate   # preflight gate (no upload)"
    @echo "  2. just release-publish-stoker     # Stage 1: publish pure crate, then wait ~2-5 min to index"
    @echo "  3. just release-dry-run-idf        # Stage 2: resolves stoker ^X.Y from crates.io"
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
# NOTE: only succeeds AFTER stoker is published to crates.io (resolves stoker ^X.Y from the index)
release-dry-run-idf:
    CARGO_TARGET_DIR="{{ idf_dir }}" cargo publish --dry-run -p rustyfarian-esp-idf-power --target {{ esp32s3_target }}

# dry-run package rustyfarian-esp-hal-power against the ESP32-C3 target (no upload; esp toolchain, no espup)
# NOTE: only succeeds AFTER stoker is published to crates.io (resolves stoker ^X.Y from the index)
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

# install the ESP-IDF toolchain via espup
setup-toolchain:
    espup install
