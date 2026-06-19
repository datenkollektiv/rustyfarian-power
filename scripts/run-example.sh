#!/usr/bin/env bash
set -euo pipefail
# run-example.sh — build and flash an idf_{chip}_{name} example
# Usage: scripts/run-example.sh <example> [idf_dir]
#   example: idf_{chip}_{name}  e.g. idf_esp32_battery, idf_esp32s3_battery
#   idf_dir: target directory for ESP-IDF builds (default: target/idf)

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./lib.sh
. "$SCRIPT_DIR/lib.sh"

if [ $# -lt 1 ]; then
    printf 'Usage: %s <example> [idf_dir]\n  example: idf_{chip}_{name}  e.g. idf_esp32_battery\n' "$0" >&2
    exit 2
fi

example="$1"
idf_dir="${2:-target/idf}"
chip=$(printf '%s' "$example" | cut -d_ -f2)

# Build the example first.
"$SCRIPT_DIR/build-example.sh" "$example" "$idf_dir"

# Derive the target path from the chip name (must match build-example.sh).
case "$chip" in
    esp32)   target="xtensa-esp32-espidf"   ;;
    esp32s3) target="xtensa-esp32s3-espidf" ;;
    *) printf 'Unknown chip "%s" in example "%s". Supported: esp32, esp32s3\n' "$chip" "$example" >&2; exit 1 ;;
esac

# Ensure the IDF-built v5.3.3 bootloader is cached.
"$SCRIPT_DIR/ensure-bootloader.sh" "$chip" "$idf_dir"

# Look up the cached bootloader using lib.sh helper.
bl=$(find_idf_bootloader "$target" "$idf_dir")

if [ -z "$bl" ]; then
    printf 'Warning: IDF-built bootloader not found; using espflash default (may fail on page-size mismatch)\n' >&2
    printf 'Flashing %s...\n' "$example"
    espflash flash --chip "$chip" --ignore-app-descriptor "$idf_dir/$target/release/examples/$example"
else
    printf 'Flashing %s with bootloader %s...\n' "$example" "$bl"
    espflash flash --chip "$chip" --bootloader "$bl" --ignore-app-descriptor "$idf_dir/$target/release/examples/$example"
fi
