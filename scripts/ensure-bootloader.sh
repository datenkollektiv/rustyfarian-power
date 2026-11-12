#!/usr/bin/env bash
set -euo pipefail
# ensure-bootloader.sh — ensure the IDF-built bootloader is cached
# Usage: scripts/ensure-bootloader.sh <chip> [idf_dir]
#   chip: esp32 | esp32s3 | c3 | c6
#
# espflash 4.x bundles its own bootloader, which rejected IDF binaries while the
# pin was ESP-IDF v5.3.3 (32 KB MMU page mismatch). The bootloader esp-idf-sys
# builds for the pinned ESP-IDF (.cargo/config.toml) always matches the app, so
# this script ensures it is cached before flashing.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./lib.sh
. "$SCRIPT_DIR/lib.sh"

if [ $# -lt 1 ]; then
    printf 'Usage: %s <chip> [idf_dir]\n  chip: esp32 | esp32s3 | c3 | c6\n' "$0" >&2
    exit 2
fi

chip="$1"
idf_dir="${2:-target/idf}"

# Map chip to IDF target and representative example
case "$chip" in
    esp32)
        idf_target="xtensa-esp32-espidf"
        idf_example="idf_esp32_battery"
        ;;
    esp32s3)
        idf_target="xtensa-esp32s3-espidf"
        idf_example="idf_esp32s3_battery"
        ;;
    c3)
        idf_target="riscv32imc-esp-espidf"
        idf_example="idf_c3_deep_sleep_gpio"
        ;;
    c6)
        idf_target="riscv32imac-esp-espidf"
        idf_example="idf_c6_deep_sleep_gpio"
        ;;
    *)
        printf 'Error: Unknown chip "%s". Supported: esp32, esp32s3, c3, c6\n' "$chip" >&2
        exit 1
        ;;
esac

bl=$(find_idf_bootloader "$idf_target" "$idf_dir")
if [ -z "$bl" ]; then
    printf 'IDF bootloader not cached for %s — building %s to populate it...\n' "$chip" "$idf_example"
    "$SCRIPT_DIR/build-example.sh" "$idf_example" "$idf_dir"
else
    printf 'Bootloader already cached for %s: %s\n' "$chip" "$bl"
fi
