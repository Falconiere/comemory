#!/usr/bin/env bash
# License + advisory scan via cargo-deny.
# Not part of check-all's always-run set — invoked from `just qa` and CI.
# Exits with a clear install message if cargo-deny isn't installed.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

if ! command -v cargo-deny >/dev/null 2>&1; then
  die "deny-check" "cargo-deny not installed (cargo install cargo-deny)"
fi
run_cargo deny check
