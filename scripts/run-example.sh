#!/usr/bin/env bash
set -euo pipefail
# run-example.sh — build and flash an idf_{chip}_{name} example
# Usage: scripts/run-example.sh <example>
#   example: idf_{chip}_{name}  e.g. idf_esp32_battery, idf_esp32s3_battery

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [ $# -lt 1 ]; then
    printf 'Usage: %s <example>\n  example: idf_{chip}_{name}  e.g. idf_esp32_battery\n' "$0" >&2
    exit 2
fi

example="$1"
chip=$(printf '%s' "$example" | cut -d_ -f2)

# Build the example first.
"$SCRIPT_DIR/build-example.sh" "$example"

# Derive the target path from the chip name (must match build-example.sh).
case "$chip" in
    esp32)   target="xtensa-esp32-espidf"   ;;
    esp32s3) target="xtensa-esp32s3-espidf" ;;
    *) printf 'Unknown chip "%s" in example "%s". Supported: esp32, esp32s3\n' "$chip" "$example" >&2; exit 1 ;;
esac

# Look up the IDF-built v5.3.3 bootloader from the build cache.
# espflash 4.x bundles an ESP-IDF v5.5.1 bootloader that rejects v5.3.3 IDF
# binaries due to a 32 KB MMU page-size mismatch. Passing the cached bootloader
# built by esp-idf-sys avoids this mismatch.
bl_candidates=( "$PWD/target/$target/release/build"/esp-idf-sys-*/out/build/bootloader/bootloader.bin )
bl=""
if [ ${#bl_candidates[@]} -gt 0 ] && [ -e "${bl_candidates[0]}" ]; then
    if [ ${#bl_candidates[@]} -gt 1 ]; then
        printf 'Error: multiple IDF-built bootloaders found for target "%s".\n' "$target" >&2
        printf 'Run `just clean` to remove stale build artefacts.\nCandidates:\n' >&2
        for cand in "${bl_candidates[@]}"; do
            printf '  %s\n' "$cand" >&2
        done
        exit 1
    fi
    bl="${bl_candidates[0]}"
fi

if [ -z "$bl" ]; then
    printf 'Warning: IDF-built bootloader not found; using espflash default (may fail on page-size mismatch)\n' >&2
    printf 'Flashing %s...\n' "$example"
    espflash flash --chip "$chip" --ignore-app-descriptor "target/$target/release/examples/$example"
else
    printf 'Flashing %s with bootloader %s...\n' "$example" "$bl"
    espflash flash --chip "$chip" --bootloader "$bl" --ignore-app-descriptor "target/$target/release/examples/$example"
fi
