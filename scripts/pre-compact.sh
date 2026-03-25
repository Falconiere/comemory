#!/usr/bin/env bash
# Pre-compaction: best-effort reminder + context snapshot
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# Persist the venv in CLAUDE_PLUGIN_DATA so it survives plugin updates.
if [ -n "${CLAUDE_PLUGIN_DATA:-}" ]; then
  mkdir -p "$CLAUDE_PLUGIN_DATA"
  export UV_PROJECT_ENVIRONMENT="${CLAUDE_PLUGIN_DATA}/.venv"
fi

UV="uv run --locked --directory $PROJECT_ROOT"

echo "## Qwick Memory — Pre-Compaction Notice"
echo ""
echo "Context compaction is about to happen."
echo "If you haven't already, call qwick_memory_session_summary now."
echo ""
echo "Current memory state:"
$UV python -m qwick_memory context --limit 5 2>/dev/null || echo "No context available."
