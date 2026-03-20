#!/usr/bin/env bash
# Auto-index on session start
set -euo pipefail

PLUGIN_ROOT="${CLAUDE_PLUGIN_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
cd "$PLUGIN_ROOT"

if [ -d "memories" ]; then
  uv run python -m qwick_rag index 2>/dev/null || true
fi
