#!/usr/bin/env bash
# Duplication scan using similarity-rs if installed; falls back to a soft warning.
#
# Colocated tests under src/**/tests/ are excluded from the scan: CORE
# excludes tests from copy-paste detection ("repeated setup there is not the
# duplication worth chasing"). similarity-rs's own --exclude flag was measured
# to NOT filter its positional PATHS list (file count stayed unchanged with it
# set), so the file list is built explicitly instead, via git ls-files
# filtered to drop any path containing /tests/.
#
# RATCHET, not an allowlist: similarity-rs has no per-pair suppression flag
# (confirmed via `similarity-rs --help`), so the 113 near-duplicate pairs
# documented in docs/dup-debt.md (pre-existing debt, found when this script's
# invocation was fixed — see that doc's intro) cannot be silenced pair-by-pair.
# Instead this gate fails only if the CURRENT duplicate-pair COUNT exceeds the
# baseline recorded in dup-baseline.txt (repo root, tracked — same convention
# as coverage-floor.txt): new duplication introduced by a future PR still
# fails the gate; the documented 113 stay grandfathered. Burning a pair down
# should lower both the baseline file and dup-debt.md's table in the same PR.
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

BASELINE_FILE="$PROJECT_ROOT/dup-baseline.txt"
if [[ ! -f "$BASELINE_FILE" ]]; then
  log_err "dup-check" "missing $BASELINE_FILE (baseline ratchet file)"
  exit 1
fi
baseline_count="$(tr -d '[:space:]' < "$BASELINE_FILE")"
if ! [[ "$baseline_count" =~ ^[0-9]+$ ]]; then
  log_err "dup-check" "$BASELINE_FILE must contain a single integer count"
  exit 1
fi

mapfile -t targets < <(
  { git ls-files 'src/*.rs' | grep -v '/tests/'; git ls-files 'scripts/*.sh'; } \
    | while IFS= read -r f; do [[ -f "$f" ]] && printf '%s\n' "$f"; done
)

# --fail-on-duplicates makes similarity-rs exit 1 whenever ANY duplicate is
# found, which is expected while the documented baseline is non-zero — so the
# exit code alone can no longer be the pass/fail signal. Capture the output
# unconditionally and let the count comparison below decide.
set +e
scan_output="$(similarity-rs --threshold 0.85 --fail-on-duplicates "${targets[@]}" 2>&1)"
set -e
scan_log="$(mktemp -t comemory-dup.XXXXXX)"
printf '%s\n' "$scan_output" > "$scan_log"

if printf '%s\n' "$scan_output" | grep -q '^No duplicate functions found!$'; then
  current_count=0
else
  current_count="$(printf '%s\n' "$scan_output" \
    | awk -F': ' '/^Total duplicate pairs found:/ { print $2 }')"
  if [[ -z "$current_count" ]]; then
    log_err "dup-check" "could not parse similarity-rs output; see $scan_log"
    exit 1
  fi
fi

if (( current_count > baseline_count )); then
  log_err "dup-check" \
    "near-duplicate count $current_count exceeds baseline $baseline_count (docs/dup-debt.md); see $scan_log"
  exit 1
fi
if (( current_count < baseline_count )); then
  log_info "dup-check" \
    "near-duplicate count $current_count is below baseline $baseline_count — lower $BASELINE_FILE to lock in the improvement"
fi
log_ok "dup-check" "near-duplicate count $current_count within baseline $baseline_count"
