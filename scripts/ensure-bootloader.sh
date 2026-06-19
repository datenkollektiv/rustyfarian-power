#!/usr/bin/env bash
set -euo pipefail
# ensure-bootloader.sh — ensure IDF-built v5.3.3 bootloader is cached
# Usage: scripts/ensure-bootloader.sh <chip> [idf_dir]
#   chip: esp32 | esp32s3
#
# espflash 4.x bundles an ESP-IDF v5.5.1 bootloader that rejects v5.3.3 IDF
# binaries (32 KB MMU page mismatch). The v5.3.3 bootloader built by esp-idf-sys
# works correctly. This script ensures it is cached before flashing.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./lib.sh
. "$SCRIPT_DIR/lib.sh"

if [ $# -lt 1 ]; then
    printf 'Usage: %s <chip> [idf_dir]\n  chip: esp32 | esp32s3\n' "$0" >&2
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
    *)
        printf 'Error: Unknown chip "%s". Supported: esp32, esp32s3\n' "$chip" >&2
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
