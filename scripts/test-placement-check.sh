#!/usr/bin/env bash
# Fail if any src/*.rs file contains an inline `#[cfg(test)] mod tests` block.
# Tests must live in the flat tests/ layout: src/a/b/c.rs -> tests/a__b__c.rs
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

cd "$PROJECT_ROOT"
# Pattern: `#[cfg(test)]` followed (on next non-empty line) by `mod tests`. We greedily detect both on adjacent lines.
hits=$(grep -RInE '^[[:space:]]*#\[cfg\(test\)\]' src/ 2>/dev/null || true)
if [[ -n "$hits" ]]; then
  log_err "test-placement-check" "inline test modules are forbidden in src/:"
  printf "%s\n" "$hits" >&2
  printf "\nMove tests to the flat tests/ layout: src/a/b/c.rs -> tests/a__b__c.rs\n" >&2
  exit 1
fi
log_ok "test-placement-check" "no inline tests in src/"
