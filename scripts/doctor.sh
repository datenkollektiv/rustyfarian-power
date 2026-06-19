#!/usr/bin/env bash
set -euo pipefail
# doctor.sh — report development tooling status for rustyfarian-power
# Usage: scripts/doctor.sh [ramdisk] [idf_dir]

ramdisk="${1:-/Volumes/RustBuilds}"
idf_dir="${2:-target/idf}"

# status <name> <state> <detail>
status() { printf '  %-16s %-9s %s\n' "$1" "$2" "$3"; }

printf 'rustyfarian-power — tooling status\n\n'

# --- Rust toolchain -------------------------------------------------------
if command -v rustc >/dev/null 2>&1; then
    status "rustc" "ok" "$(rustc --version 2>/dev/null)"
else
    status "rustc" "MISSING" "install Rust, then the esp toolchain via espup"
fi

if command -v cargo >/dev/null 2>&1; then
    status "cargo" "ok" "$(cargo --version 2>/dev/null)"
else
    status "cargo" "MISSING" "install Rust"
fi

if command -v rustup >/dev/null 2>&1 && rustup toolchain list 2>/dev/null | grep -q '^esp'; then
    status "esp toolchain" "ok" "rustup 'esp' channel present (Xtensa Rust fork)"
else
    status "esp toolchain" "MISSING" "run: espup install"
fi

# --- Build / flash tools --------------------------------------------------
if command -v just >/dev/null 2>&1; then
    status "just" "ok" "$(just --version 2>/dev/null)"
else
    status "just" "MISSING" "install just (the task runner running this)"
fi

if command -v espflash >/dev/null 2>&1; then
    status "espflash" "ok" "$(espflash --version 2>/dev/null | head -1)"
else
    status "espflash" "MISSING" "run: cargo install espflash  (needed for: just flash/monitor)"
fi

if command -v ldproxy >/dev/null 2>&1; then
    status "ldproxy" "ok" "$(command -v ldproxy)"
else
    status "ldproxy" "MISSING" "run: cargo install ldproxy  (the ESP-IDF linker wrapper)"
fi

if command -v cargo-deny >/dev/null 2>&1; then
    status "cargo-deny" "ok" "$(cargo-deny --version 2>/dev/null)"
else
    status "cargo-deny" "optional" "run: cargo install cargo-deny  (needed for: just deny)"
fi

# --- ESP-IDF environment --------------------------------------------------
if [ -d "$HOME/.espressif" ]; then
    status "esp-idf tools" "ok" "$HOME/.espressif (ESP_IDF_TOOLS_INSTALL_DIR=global)"
else
    status "esp-idf tools" "MISSING" "populated on first ESP-IDF build (kept off the RAM disk)"
fi

if [ -f "$HOME/export-esp.sh" ]; then
    status "export-esp.sh" "ok" "source ~/export-esp.sh before ESP-IDF builds"
else
    status "export-esp.sh" "--" "not found (espup writes it; needed to put the Xtensa GCC on PATH)"
fi

# --- Build target directories / RAM disk ----------------------------------
if [ -d "$ramdisk" ]; then
    if [ -d "$ramdisk/targets/idf" ]; then
        status "ramdisk" "ok" "$ramdisk"
        status "idf target" "ramdisk" "$idf_dir"
    else
        status "ramdisk" "PARTIAL" "$ramdisk (subdir missing — run: just ramdisk attach)"
        status "idf target" "fallback" "$idf_dir"
    fi
else
    status "ramdisk" "off" "optional — run: just ramdisk attach  (faster builds, spares the SSD)"
    status "idf target" "disk" "$idf_dir"
fi
