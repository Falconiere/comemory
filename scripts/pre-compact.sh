#!/usr/bin/env bash
# Pre-compaction: best-effort reminder + context snapshot
set -euo pipefail

PLUGIN_ROOT="${CLAUDE_PLUGIN_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
cd "$PLUGIN_ROOT"

echo "## Qwick Memory — Pre-Compaction Notice"
echo ""
echo "Context compaction is about to happen."
echo "If you haven't already, call qwick_memory_session_summary now."
echo ""
echo "Current memory state:"
uv run python -m qwick_memory context --limit 5 2>/dev/null || echo "No context available."
