#!/usr/bin/env bash
set -euo pipefail
# flash.sh — route an idf_{chip}_{name} example to run-example.sh
# Usage: scripts/flash.sh <example>
#   example: idf_{chip}_{name}  e.g. idf_esp32_battery, idf_esp32s3_battery

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [ $# -lt 1 ]; then
    printf 'Usage: %s <example>\n  example: idf_{chip}_{name}  e.g. idf_esp32_battery\n' "$0" >&2
    exit 2
fi

example="$1"
prefix=$(printf '%s' "$example" | cut -d_ -f1)

case "$prefix" in
    idf) "$SCRIPT_DIR/run-example.sh" "$example" ;;
    *) printf 'Unknown driver prefix "%s". Supported: idf (e.g., idf_esp32_battery)\n' "$prefix" >&2; exit 1 ;;
esac
