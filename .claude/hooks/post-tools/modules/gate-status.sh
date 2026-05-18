#!/usr/bin/env bash
# Track exit codes of recognized quality commands and persist to
# .claude/tmp/quality-gate-status.json. Emit a PostToolUse.additionalContext
# warning when a gate fails so the agent surfaces the issue next turn.

: "${tool_name:=}"; : "${input:=}"; : "${PROJECT_ROOT:=$(pwd)}"
[[ "$tool_name" != "Bash" && "$tool_name" != "Shell" ]] && exit 0

HOOK_LIB="$(cd "$(dirname "$0")/../.." && pwd)/lib/common.sh"
# shellcheck source=../../lib/common.sh
source "$HOOK_LIB"

GATE_DIR="$PROJECT_ROOT/.claude/tmp"
GATE_FILE="$GATE_DIR/quality-gate-status.json"
mkdir -p "$GATE_DIR"

command=$(echo "$input" | jq -r '.tool_input.command // ""' 2>/dev/null || echo "")
exit_code=$(echo "$input" | jq -r '.tool_response.metadata.exit_code // .tool_response.exit_code // empty' 2>/dev/null || echo "")

if ! echo "$command" | grep -qE '(bash[[:space:]]+scripts/(check-all|fmt-check|type-check|lint-check|test-run|test-placement-check|tests-mirror-check|no-bypass-check|module-size-check|typos-check|deny-check|dup-check|e2e)\.sh|just[[:space:]]+(check|qa|test|e2e))'; then
  exit 0
fi

ts="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
if [[ "$exit_code" =~ ^[0-9]+$ && "$exit_code" -ne 0 ]]; then
  jq -n --arg s "failing" --arg c "$command" --arg e "$exit_code" --arg t "$ts" \
    '{status:$s, command:$c, exit_code:($e|tonumber), updatedAt:$t, source:"gate-status-hook"}' > "$GATE_FILE"
  post_context "Quality gate FAILING. Fix before continuing."$'\n'"Failed: $command (exit $exit_code)"
  exit 0
fi

if [[ "$exit_code" == "0" ]]; then
  jq -n --arg s "passing" --arg c "$command" --arg t "$ts" \
    '{status:$s, command:$c, updatedAt:$t}' > "$GATE_FILE"
fi
exit 0
