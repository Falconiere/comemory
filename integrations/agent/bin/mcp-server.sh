#!/usr/bin/env bash
# Launch `comemory mcp` for hosts whose plugin manifest cannot carry an
# absolute binary path (muse). `comemory install` rewrites ../.mcp.json on
# every install with the installing binary's absolute path — the same file
# Claude Code and Codex read — so the server always matches the hooks and
# bundle it was installed with. Falls back to `comemory` on PATH only when
# that manifest is unreadable or names no executable.
set -euo pipefail
root="$(cd "${BASH_SOURCE%/*}/.." && pwd)"
bin=""
if command -v jq >/dev/null 2>&1; then
  bin=$(jq -r '.mcpServers.comemory.command // empty' "$root/.mcp.json" 2>/dev/null) || bin=""
fi
if [ -z "$bin" ] || [ ! -x "$bin" ]; then
  bin=comemory
fi
exec "$bin" mcp "$@"
