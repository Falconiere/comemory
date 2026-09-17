#!/usr/bin/env bash
# Assert every crate in the resolved dependency tree carries a license that
# deny.toml's `[licenses] allow` list permits.
#
# This is a standalone fallback for `scripts/deny-check.sh`, which needs
# `cargo-deny` installed and exits 1 when it is not. cargo-deny remains the
# richer gate (advisories, bans, sources); this one covers the license axis
# alone, using data cargo itself resolves, so a machine without cargo-deny can
# still prove the licence question rather than skipping it.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"
cd "$PROJECT_ROOT"

STEP="license-audit"
require_cmd python3
require_cmd cargo "https://rustup.rs"

# The SPDX expression handling is the only non-trivial part, so its own
# real-corpus test runs first: a parser bug that wrongly ALLOWS a license
# would otherwise make the audit below a silent false green.
log_info "$STEP" "verifying the SPDX parser against the real license corpus"
python3 "$HERE/lib/test_license_audit.py" >/dev/null

log_info "$STEP" "auditing resolved crate licenses against deny.toml"
cargo metadata --format-version 1 --all-features 2>/dev/null \
  | python3 "$HERE/lib/license_audit.py" "$PROJECT_ROOT/deny.toml"
log_ok "$STEP" "every crate license is on deny.toml's allow list"
