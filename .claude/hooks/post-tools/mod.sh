#!/usr/bin/env bash
# Post-tool-use dispatcher.
set -uo pipefail

input=$(cat 2>/dev/null || echo "{}")
tool_name=$(echo "$input" | jq -r '.tool_name // ""' 2>/dev/null || echo "")
PROJECT_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
export PATH="$PROJECT_ROOT/target/debug:$PATH"
export input tool_name PROJECT_ROOT

HOOK_DIR="$(cd "$(dirname "$0")" && pwd)/modules"
for script in "$HOOK_DIR"/*.sh; do
  [[ ! -f "$script" ]] && continue
  result=$(echo "$input" | bash "$script" 2>/dev/null)
  if [[ -n "$result" ]]; then
    echo "$result"
    exit 0
  fi
done
exit 0
