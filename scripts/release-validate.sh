#!/usr/bin/env bash
set -euo pipefail
# release-validate.sh — pre-flight validation for the crates.io release (no actual publish)
# Usage: scripts/release-validate.sh
#   RELEASE_ALLOW_DIRTY=1 scripts/release-validate.sh   # local iteration: skip the clean-tree guard
#
# Runs the full release gate: clean-tree guard, version lockstep, `just verify`,
# package-content checks, a `cargo publish --dry-run` for the host-buildable
# `stoker` crate, the `cargo deny` policy gate, and an advisory `cargo audit`.
# See release-plan.md for the full publication sequence.
#
# This repo's default toolchain is the `esp` channel (rust-toolchain.toml), so
# plain `cargo` already drives the Espressif Xtensa fork — no `cargo +esp` needed.
#
# CHANGELOG.md is updated (move [Unreleased] -> [X.Y.Z]) in the release commit
# BEFORE running this and tagging — it is NOT a post-publish step.
#
# PREREQUISITE: the battery-monitor -> stoker + rustyfarian-esp-idf-power split
# (release-plan.md, Phase 0) must be complete before this script can pass.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

CRATES=(stoker rustyfarian-esp-idf-power)

# Required tooling — fail fast and clearly rather than mid-step.
for tool in jq just cargo git; do
    command -v "$tool" >/dev/null 2>&1 ||
        { echo "ERROR: '$tool' is required but not found in PATH" >&2; exit 1; }
done
cargo audit --version >/dev/null 2>&1 ||
    { echo "ERROR: cargo-audit is required (cargo install cargo-audit)" >&2; exit 1; }
cargo deny --version >/dev/null 2>&1 ||
    { echo "ERROR: cargo-deny is required (cargo install cargo-deny)" >&2; exit 1; }

host_target="$("$SCRIPT_DIR/host-target.sh")"

# Per-run log files (mktemp avoids collisions across users/concurrent runs).
verify_log="$(mktemp -t release-verify.XXXXXX)"
dryrun_log="$(mktemp -t release-dryrun-stoker.XXXXXX)"
deny_log="$(mktemp -t release-deny.XXXXXX)"
audit_log="$(mktemp -t release-audit.XXXXXX)"

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "Release Validation — 0.1.0 lockstep (stoker + rustyfarian-esp-idf-power)"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

echo "[1/6] Verifying clean working tree (tracked files)..."
# A release gate validates committed state. Untracked scratch dirs (review-queue/,
# tmp/) are expected and tolerated — the cargo package/publish commands below run
# with --allow-dirty solely so those untracked files don't block packaging.
# Set RELEASE_ALLOW_DIRTY=1 to skip this guard during local iteration.
if [ "${RELEASE_ALLOW_DIRTY:-0}" != "1" ]; then
    if ! git diff --quiet || ! git diff --cached --quiet; then
        echo "  ERROR: uncommitted changes to tracked files." >&2
        echo "         Commit the release prep (CHANGELOG move, metadata) first," >&2
        echo "         or re-run with RELEASE_ALLOW_DIRTY=1 for local iteration." >&2
        exit 1
    fi
    echo "  OK — no uncommitted changes to tracked files"
else
    echo "  SKIPPED — RELEASE_ALLOW_DIRTY=1 (local iteration)"
fi
echo ""

echo "[2/6] Verifying version consistency..."
metadata=$(cargo metadata --format-version 1 2>/dev/null || true)
stoker_ver=$(echo "$metadata" | jq -r '.packages[] | select(.name == "stoker") | .version')
idf_ver=$(echo "$metadata" | jq -r '.packages[] | select(.name == "rustyfarian-esp-idf-power") | .version')
if [ -z "$stoker_ver" ] || [ -z "$idf_ver" ]; then
    echo "  ERROR: could not resolve crate versions — is the crate split complete? (release-plan.md Phase 0)" >&2
    exit 1
fi
if [ "$stoker_ver" != "$idf_ver" ]; then
    echo "  ERROR: version mismatch — stoker=$stoker_ver idf=$idf_ver" >&2
    exit 1
fi
echo "  OK — both crates at version $stoker_ver"
echo ""

echo "[3/6] Running 'just verify'..."
if just verify >"$verify_log" 2>&1; then
    echo "  OK — fmt-check, check, clippy, host tests"
else
    echo "  FAIL — see $verify_log" >&2
    tail -20 "$verify_log" >&2
    exit 1
fi
echo ""

echo "[4/6] Validating package contents (exact README + dual-license files)..."
for crate in "${CRATES[@]}"; do
    listing=$(cargo package --list -p "$crate" --allow-dirty 2>&1)
    missing=()
    echo "$listing" | grep -qx "LICENSE-MIT" || missing+=("LICENSE-MIT")
    echo "$listing" | grep -qx "LICENSE-APACHE" || missing+=("LICENSE-APACHE")
    echo "$listing" | grep -qx "README.md" || missing+=("README.md")
    if [ ${#missing[@]} -gt 0 ]; then
        echo "  ERROR: $crate package is missing: ${missing[*]}" >&2
        exit 1
    fi
    echo "  OK — $crate: $(echo "$listing" | wc -l | tr -d ' ') files, LICENSE-MIT + LICENSE-APACHE + README.md"
done
echo ""

echo "[5/6] cargo publish --dry-run (stoker — host-buildable, full verify)..."
if cargo publish --dry-run -p stoker --target "$host_target" --all-features --allow-dirty >"$dryrun_log" 2>&1; then
    echo "  OK — stoker packages and verify-builds"
else
    echo "  FAIL — see $dryrun_log" >&2
    tail -20 "$dryrun_log" >&2
    exit 1
fi
# rustyfarian-esp-idf-power depends on `stoker ^0.1`. A `cargo publish --dry-run`
# for it resolves stoker against the crates.io index (the published manifest drops
# the path), which only succeeds AFTER stoker is published — so a standalone dry-run
# is not possible here. Its packaging/contents are validated above in [4/6] via
# `cargo package --list`; its real publish --dry-run happens as the ordered publish
# proceeds (publish stoker first, which unblocks it). See release-plan.md.
echo "  NOTE — rustyfarian-esp-idf-power: full publish --dry-run requires stoker on"
echo "         crates.io first; validated via package --list above and as part of the"
echo "         ordered publish (just release-dry-run-idf, then just release-publish-idf)."
echo ""

echo "[6/6] Dependency policy (cargo deny, blocking) + advisories (cargo audit, advisory)..."
if just deny >"$deny_log" 2>&1; then
    echo "  OK — cargo deny: licenses, advisories, and bans within deny.toml policy"
else
    echo "  FAIL — cargo deny policy violation; see $deny_log" >&2
    tail -20 "$deny_log" >&2
    exit 1
fi
if cargo audit >"$audit_log" 2>&1; then
    echo "  OK — cargo audit: no known vulnerabilities"
else
    echo "  NOTE — cargo audit reported findings (advisory); review $audit_log." >&2
    echo "         The blocking policy gate is cargo deny above (it honours deny.toml)." >&2
fi
echo ""

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "PRE-FLIGHT VALIDATION PASSED — ready to publish v$stoker_ver"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "Publish via the just recipes, in staged dependency order (clean tree + CARGO_REGISTRY_TOKEN):"
echo "  Stage 1: just release-publish-stoker         # wait ~2-5 min to index"
echo "  Stage 2: just release-dry-run-idf            # now resolves stoker ^0.1 from the index"
echo "  Stage 3: just release-publish-idf            # cargo publish --target xtensa-esp32s3-espidf (esp toolchain)"
echo ""
echo "  rustyfarian-esp-idf-power verify-builds against its real cross-target"
echo "  (xtensa-esp32s3-espidf), NOT the host and NOT --no-verify; it needs espup."
echo "  See release-plan.md."
echo ""
echo "Then: git tag -a v$stoker_ver -m \"v$stoker_ver\" && git push --tags, and cut the GitHub release."
echo "(CHANGELOG.md was already moved [Unreleased] -> [$stoker_ver] in the release commit.)"
echo ""
echo "See release-plan.md for the full checklist and troubleshooting."
