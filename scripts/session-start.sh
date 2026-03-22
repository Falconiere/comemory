#!/usr/bin/env bash
# Session start: auto-index + output context for Claude
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
