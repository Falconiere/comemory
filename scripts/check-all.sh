#!/usr/bin/env bash
# Run every quality gate. Exit 1 on first failure.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

GATES=(
  fmt-check
  type-check
  lint-check
  test-placement-check
  no-bypass-check
  module-size-check
  tests-mirror-check
  typos-check
  cli-docs-check
)

failed=()
for g in "${GATES[@]}"; do
  log_info "$g" "running"
  if bash "$HERE/$g.sh"; then
    log_ok "$g"
  else
    log_err "$g" "failed"
    failed+=("$g")
  fi
done

if (( ${#failed[@]} > 0 )); then
  log_err "check-all" "${#failed[@]} gate(s) failed: ${failed[*]}"
  exit 1
fi
log_ok "check-all" "all gates passed"
