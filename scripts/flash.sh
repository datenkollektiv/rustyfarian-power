#!/usr/bin/env bash
set -euo pipefail
# flash.sh — route an idf_{chip}_{name} example to run-example.sh
# Usage: scripts/flash.sh <example> [idf_dir]
#   example: idf_{chip}_{name}  e.g. idf_esp32_battery, idf_esp32s3_battery
#   idf_dir: target directory for ESP-IDF builds (default: target/idf)

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [ $# -lt 1 ]; then
    printf 'Usage: %s <example> [idf_dir]\n  example: idf_{chip}_{name}  e.g. idf_esp32_battery\n' "$0" >&2
    exit 2
fi

example="$1"
idf_dir="${2:-target/idf}"
prefix=$(printf '%s' "$example" | cut -d_ -f1)

case "$prefix" in
    idf) "$SCRIPT_DIR/run-example.sh" "$example" "$idf_dir" ;;
    *) printf 'Unknown driver prefix "%s". Supported: idf (e.g., idf_esp32_battery)\n' "$prefix" >&2; exit 1 ;;
esac
