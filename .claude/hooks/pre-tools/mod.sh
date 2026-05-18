#!/usr/bin/env bash
# Pre-tool-use dispatcher. Runs every module; first non-empty stdout wins.

input=$(cat)
tool_name=$(echo "$input" | jq -r '.tool_name // ""' 2>/dev/null || echo "")
export input tool_name

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
