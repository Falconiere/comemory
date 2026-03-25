#!/usr/bin/env bash
# Launch qwick-memory MCP server from any working directory.
# Resolves the project root from this script's location, so it works
# whether invoked via CLAUDE_PLUGIN_ROOT or from an arbitrary CWD.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# Persist the venv in CLAUDE_PLUGIN_DATA so it survives plugin updates.
# Without this, every auto-update destroys the cache-dir .venv and forces
# a full reinstall of lancedb/fastembed (~67 packages) on next startup.
if [ -n "${CLAUDE_PLUGIN_DATA:-}" ]; then
  mkdir -p "$CLAUDE_PLUGIN_DATA"
  export UV_PROJECT_ENVIRONMENT="${CLAUDE_PLUGIN_DATA}/.venv"
fi

exec uv run --locked --directory "$PROJECT_ROOT" python -m qwick_memory.server
