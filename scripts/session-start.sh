#!/usr/bin/env bash
# Session start: auto-index + output context for Claude
set -euo pipefail

PLUGIN_ROOT="${CLAUDE_PLUGIN_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
cd "$PLUGIN_ROOT"

# Auto-index
if [ -d "memories" ]; then
  uv run python -m qwick_rag index 2>/dev/null || true
fi

# Output context for Claude
echo "## Qwick Memory — Session Context"
echo ""
uv run python -m qwick_rag context 2>/dev/null || echo "No prior context found."
