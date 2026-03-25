#!/usr/bin/env bash
# Session start: auto-index + output context + protocol reminder for Claude
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# Persist the venv in CLAUDE_PLUGIN_DATA so it survives plugin updates.
if [ -n "${CLAUDE_PLUGIN_DATA:-}" ]; then
  mkdir -p "$CLAUDE_PLUGIN_DATA"
  export UV_PROJECT_ENVIRONMENT="${CLAUDE_PLUGIN_DATA}/.venv"
fi

UV="uv run --locked --directory $PROJECT_ROOT"

# Auto-migrate (flatten nested dirs, rebuild index if model changed)
$UV python -m qwick_memory migrate 2>/dev/null || true

# Auto-index (incremental — picks up any new memories since last session)
$UV python -m qwick_memory index 2>/dev/null || true

# Output context for Claude
echo "## Qwick Memory — Session Context"
echo ""
$UV python -m qwick_memory context 2>/dev/null || echo "No prior context found."

# Decision guide footer (high-attention position at end of session start)
echo ""
echo "---"
echo "Memory Protocol Active:"
echo "-> SEARCH before answering questions about prior work, PRs, decisions, or history"
echo "-> SAVE after decisions, bug fixes, discoveries, conventions, preferences"
echo "-> SUMMARIZE before ending session"
echo ""
echo "REMINDER: save decisions, bugs, and discoveries to qwick-memory. Always specify repo."
