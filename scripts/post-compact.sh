#!/usr/bin/env bash
# Post-compaction: restore context from memories
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# Persist the venv in CLAUDE_PLUGIN_DATA so it survives plugin updates.
if [ -n "${CLAUDE_PLUGIN_DATA:-}" ]; then
  mkdir -p "$CLAUDE_PLUGIN_DATA"
  export UV_PROJECT_ENVIRONMENT="${CLAUDE_PLUGIN_DATA}/.venv"
fi

UV="uv run --locked --directory $PROJECT_ROOT"

echo "## Qwick Memory — Context Restored After Compaction"
echo ""
echo "Context was just compacted. Here are your recent memories:"
echo ""
$UV python -m qwick_memory context 2>/dev/null || echo "No prior context found."
