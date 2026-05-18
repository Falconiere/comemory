#!/usr/bin/env bash
# Real binary, happy-path smoke. Stand-in for the full e2e flow.
# Slices that land later (save/index-code/context) extend the assertions here.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

QWICK_HOME=$(mktemp -d)
trap 'rm -rf "$QWICK_HOME"' EXIT

export QWICK_MEMORY_DATA_DIR="$QWICK_HOME/.qwick-memory"
cd "$PROJECT_ROOT"
cargo build --release --quiet
BIN="$PROJECT_ROOT/target/release/qwick-memory"

"$BIN" --version | grep -q "qwick-memory" || die "e2e" "version check failed"
log_ok "e2e" "version smoke passed"
