#!/usr/bin/env bash
# Post-compaction: restore context from memories
set -euo pipefail

PLUGIN_ROOT="${CLAUDE_PLUGIN_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
cd "$PLUGIN_ROOT"

echo "## Qwick Memory — Context Restored After Compaction"
echo ""
echo "Context was just compacted. Here are your recent memories:"
echo ""
uv run python -m qwick_rag context 2>/dev/null || echo "No prior context found."
