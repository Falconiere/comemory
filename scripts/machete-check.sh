#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib/common.sh
source "$HERE/lib/common.sh"
cd "$PROJECT_ROOT"
if ! command -v cargo-machete >/dev/null 2>&1; then
  log_info "machete-check" "cargo-machete not installed; skipping"
  exit 0
fi
cargo machete
