#!/usr/bin/env bash
# Duplication scan using similarity-rs if installed; falls back to a soft warning.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"
cd "$PROJECT_ROOT"
if ! command -v similarity-rs >/dev/null 2>&1; then
  if [[ -n "${CI:-}" ]]; then
    log_err "dup-check" "similarity-rs required in CI (cargo install similarity-rs)"
    exit 1
  fi
  log_info "dup-check" "similarity-rs not installed; skipping locally"
  exit 0
fi
# Threshold 0.85 ≈ near-clones; treat any hit as failure.
if similarity-rs --min-similarity 0.85 --paths src/ scripts/ | tee /tmp/qwick-dup.txt | grep -qE '^Similar'; then
  log_err "dup-check" "near-duplicate blocks detected; see /tmp/qwick-dup.txt"
  exit 1
fi
log_ok "dup-check" "no near-duplicates above threshold"
