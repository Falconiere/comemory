#!/usr/bin/env bash
# Launch qwick-memory MCP server from any working directory.
# Resolves the project root from this script's location, so it works
# whether invoked via CLAUDE_PLUGIN_ROOT or from an arbitrary CWD.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

exec uv run --locked --directory "$PROJECT_ROOT" python -m qwick_memory.server
