#!/usr/bin/env bash
# Sourced by hook modules. Provides JSON parsing helpers and deny emitters.

set -euo pipefail

parse_tool_name() { jq -r '.tool_name // ""' 2>/dev/null || echo ""; }

# Args: $1 = reason text
deny_pre() {
  jq -n --arg r "$1" '{
    "hookSpecificOutput": {
      "hookEventName": "PreToolUse",
      "permissionDecision": "deny",
      "permissionDecisionReason": $r
    }
  }'
}

# Args: $1 = additional context string
post_context() {
  jq -n --arg c "$1" '{
    "hookSpecificOutput": {
      "hookEventName": "PostToolUse",
      "additionalContext": $c
    }
  }'
}
