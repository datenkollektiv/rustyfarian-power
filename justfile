# Rustyfarian Power — development tasks
#
# The workspace defaults to the xtensa-esp32s3-espidf target via
# .cargo/config.toml, so host-side recipes pass --target explicitly to
# override it and disable the esp-idf feature.
#
# Run `just setup-toolchain` and `just setup-cargo-config` for first-time setup.

host_target := `scripts/host-target.sh`
host_flags  := "--no-default-features --target " + host_target
doc_flags   := "--no-default-features --target " + host_target + " --no-deps"
esp32s3_target := "xtensa-esp32s3-espidf"
esp32_target   := "xtensa-esp32-espidf"

# list available recipes (default)
_default:
    @just --list

# --- Build & Check --------------------------------------------------------

# check platform-independent code (no ESP toolchain required)
check:
    cargo check {{ host_flags }}

# check all code including ESP-IDF hardware implementations (requires espup)
check-all:
    cargo check

# check battery-monitor for the ESP32 target (Adafruit Feather V2, requires espup)
check-esp32:
    MCU=esp32 cargo check -p battery-monitor --target {{ esp32_target }}

# verify device-side rustdoc snippets type-check for the ESP32 target (requires espup)
# run this whenever touching rust,ignore doc snippets or esp-idf-gated code
check-docs-esp32:
    MCU=esp32 cargo check -p battery-monitor --target {{ esp32_target }} --features esp-idf

# build platform-independent code (no ESP toolchain required)
build:
    cargo build {{ host_flags }}

# build all code including ESP-IDF hardware implementations (requires espup)
build-all:
    cargo build

# --- Examples -------------------------------------------------------------

# build a named example without flashing — chip inferred from idf_{chip}_{name} prefix
build-example example:
    scripts/build-example.sh "{{ example }}"

# build and flash a named example — chip inferred from idf_{chip}_{name} prefix
flash example:
    scripts/flash.sh "{{ example }}"

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
    cargo clippy -- -D warnings

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

# --- Maintenance ----------------------------------------------------------

# check dependency licenses, advisories, and bans
deny:
    cargo deny check

# update dependencies
update:
    cargo update

# clean build artifacts
clean:
    cargo clean

# --- Composite ------------------------------------------------------------

# full pre-commit verification: format, check, lint, test (modifies files — local use only)
pre-commit: fmt check clippy test

# non-modifying full verification: fails on any anomaly
verify:
    @cargo fmt -- --check || (printf '\nFormatting issues found — run `just pre-commit` to auto-fix.\n' >&2 && exit 1)
    cargo check {{ host_flags }}
    cargo clippy {{ host_flags }} -- -D warnings
    cargo test {{ host_flags }}

# CI-equivalent verification (non-modifying): format check, deny, check, lint, test
ci: fmt-check deny check clippy test

# --- Setup ----------------------------------------------------------------

# copy the cargo config template for first-time setup
setup-cargo-config:
    cp .cargo/config.toml.dist .cargo/config.toml

# install the ESP-IDF toolchain via espup
setup-toolchain:
    espup install
