#!/usr/bin/env bash
# Run the umbrella quality gate. Stop on the first failure.
#
# Cheap repository checks run before compilation. Clippy also type-checks every
# target, so do not run type-check.sh again here. Keep tool-dependent QA gates
# (duplication, deny, machete, install lifecycle) in just qa and CI.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

GATES=(
  fmt-check
  typos-check
  guardrails-check
  architecture-check
  store-chokepoint-check
  migration-check
  replication-coverage-check
  lint-check
  cli-docs-check
  http-docs-check
)

started=$SECONDS
for g in "${GATES[@]}"; do
  gate_started=$SECONDS
  log_info "$g" "running"
  if bash "$HERE/$g.sh"; then
    log_ok "$g" "$((SECONDS - gate_started))s"
  else
    status=$?
    log_err "$g" "failed after $((SECONDS - gate_started))s"
    log_err "check-all" "stopped after $((SECONDS - started))s"
    exit "$status"
  fi
done
log_ok "check-all" "all gates passed in $((SECONDS - started))s"
