#!/usr/bin/env bash
# Stop hook: run fast gates and surface failures.
set -uo pipefail

PROJECT_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$PROJECT_ROOT" || exit 0

failed=()
for gate in fmt-check test-placement-check no-bypass-check module-size-check; do
  if ! out=$(bash "scripts/$gate.sh" 2>&1); then
    failed+=("$gate")
    echo "[session-end] FAIL $gate"
    echo "$out" | tail -10
  fi
done

if (( ${#failed[@]} > 0 )); then
  printf '\n[session-end] %s gate(s) failed: %s\n' "${#failed[@]}" "${failed[*]}"
  exit 1
fi
echo "[session-end] all fast gates passed"
