#!/usr/bin/env bash
set -euo pipefail
# build-example.sh — build an idf_{chip}_{name} example without flashing
# Usage: scripts/build-example.sh <example> [idf_dir]
#   example: idf_{chip}_{name}  e.g. idf_esp32_battery, idf_esp32s3_battery, idf_c3_deep_sleep_gpio, idf_s3_deep_sleep_gpio
#   idf_dir: target directory for ESP-IDF builds (default: target/idf)

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [ $# -lt 1 ]; then
    printf 'Usage: %s <example> [idf_dir]\n  example: idf_{chip}_{name}  e.g. idf_esp32_battery\n' "$0" >&2
    exit 2
fi

example="$1"
idf_dir="${2:-target/idf}"
prefix=$(printf '%s' "$example" | cut -d_ -f1)
chip=$(printf '%s' "$example" | cut -d_ -f2)

case "$prefix" in
    idf) ;;
    *) printf 'Unknown driver prefix "%s" in example "%s". Supported: idf\n' "$prefix" "$example" >&2; exit 1 ;;
esac

case "$chip" in
    esp32)   target="xtensa-esp32-espidf"   ; mcu="esp32"   ;;
    esp32s3) target="xtensa-esp32s3-espidf" ; mcu="esp32s3" ;;
    s3)      target="xtensa-esp32s3-espidf" ; mcu="esp32s3" ;;
    c3)      target="riscv32imc-esp-espidf" ; mcu="esp32c3" ;;
    *) printf 'Unknown chip "%s" in example "%s". Supported: esp32, esp32s3, s3, c3\n' "$chip" "$example" >&2; exit 1 ;;
esac

# Set up the Xtensa GCC toolchain if not already in PATH (RISC-V chips do not need it).
if [ "$chip" != "c3" ]; then
    # shellcheck source=./xtensa-toolchain.sh
    . "$SCRIPT_DIR/xtensa-toolchain.sh"
    setup_xtensa_toolchain
fi

printf 'Building %s for %s...\n' "$example" "$target"
export CARGO_TARGET_DIR="$idf_dir"
MCU="$mcu" cargo build --release \
    --target "$target" \
    --example "$example" \
    -p rustyfarian-esp-idf-power
