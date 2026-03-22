#!/usr/bin/env bash
# Session start: auto-index + output context + protocol reminder for Claude
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
UV="uv run --directory $PROJECT_ROOT"

# Auto-index
if [ -d "$PROJECT_ROOT/memories" ]; then
  $UV python -m qwick_memory index 2>/dev/null || true
fi

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
