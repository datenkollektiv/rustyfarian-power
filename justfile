# Rustyfarian Power — development tasks
#
# The workspace defaults to the xtensa-esp32s3-espidf target via
# .cargo/config.toml, so host-side recipes pass --target explicitly to
# override it and disable the esp-idf feature.
#
# Run `just setup-toolchain` and `just setup-cargo-config` for first-time setup.

host_target := `host=$(rustc -vV 2>/dev/null | grep '^host:' | awk '{print $2}'); if [ -z "$host" ]; then printf 'Error: Failed to determine rustc host target.\n' >&2; exit 1; fi; echo "$host"`
host_flags  := "--no-default-features --target " + host_target

# list available recipes (default)
_default:
    @just --list

# check platform-independent code (no ESP toolchain required)
check:
    cargo check {{ host_flags }}

# check all code including ESP-IDF hardware implementations (requires espup)
check-all:
    cargo check

# build platform-independent code (no ESP toolchain required)
build:
    cargo build {{ host_flags }}

# build all code including ESP-IDF hardware implementations (requires espup)
build-all:
    cargo build

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

# build rustdoc for platform-independent code
doc:
    cargo doc --no-default-features --target {{ host_target }} --no-deps

# build and open docs in browser
doc-open:
    cargo doc --no-default-features --target {{ host_target }} --no-deps --open

# check dependency licenses, advisories, and bans
deny:
    cargo deny check

# update dependencies
update:
    cargo update

# clean build artifacts
clean:
    cargo clean

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

# copy the cargo config template for first-time setup
setup-cargo-config:
    cp .cargo/config.toml.dist .cargo/config.toml

# install the ESP-IDF toolchain via espup
setup-toolchain:
    espup install
