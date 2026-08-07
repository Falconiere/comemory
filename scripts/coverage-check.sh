#!/usr/bin/env bash
# Run the test suite under cargo-llvm-cov. Enforce coverage-floor.txt when
# present (line coverage); otherwise run report-only so a fresh tree (no floor
# committed yet) does not red-wall. Mirrors the spec's "report-only until
# baseline exists" rule.
#
# Colocated unit tests now compile into the lib target itself, so without an
# ignore they would count 100%-covered test code as covered library code and
# inflate the number. Excluded from both the enforcing and report-only runs.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"
cd "$PROJECT_ROOT"

require_cmd cargo-nextest "cargo install cargo-nextest"
require_cmd cargo-llvm-cov "cargo install cargo-llvm-cov"

floor_file="$PROJECT_ROOT/coverage-floor.txt"
if [[ -f "$floor_file" ]]; then
  floor="$(tr -d '[:space:]' < "$floor_file")"
  log_info "coverage-check" "enforcing >= ${floor}% line coverage"
  run_cargo llvm-cov nextest --all-features \
    --ignore-filename-regex '(^|/)tests/' --fail-under-lines "$floor"
  log_ok "coverage-check" "line coverage >= ${floor}%"
else
  log_info "coverage-check" "no coverage-floor.txt — report-only"
  run_cargo llvm-cov nextest --all-features \
    --ignore-filename-regex '(^|/)tests/' --summary-only
  log_ok "coverage-check" "report-only (write coverage-floor.txt to enforce)"
fi
