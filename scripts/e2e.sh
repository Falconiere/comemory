#!/usr/bin/env bash
# Real binary, happy-path smoke. Stand-in for the full e2e flow.
# Slices that land later (save/index-code/context) extend the assertions here.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

QWICK_HOME=$(mktemp -d)
trap 'rm -rf "$QWICK_HOME"' EXIT

export COMEMORY_DATA_DIR="$QWICK_HOME/.comemory"
cd "$PROJECT_ROOT"
cargo build --release --quiet
BIN="$PROJECT_ROOT/target/release/comemory"

"$BIN" --version | grep -q "comemory" || die "e2e" "version check failed"
log_ok "e2e" "version smoke passed"
