#!/usr/bin/env bash
# SessionStart hook — publish this project's comemory memory count for the statusline.
#
# Counts memories for the MAIN-repo key on each session start (startup/resume/
# clear/compact) and writes a small marker
# the statusline reads to render a [COMEMORY:N] badge. The key is derived from
# git-common-dir so worktrees share the main repo's memory scope (a bare worktree
# toplevel basename would incorrectly scope to the worktree name and read 0).
#
# Failed refreshes preserve the existing marker. Calls have a five-second
# deadline when timeout/gtimeout is available.
set -uo pipefail

input="$(cat 2>/dev/null)"   # consume stdin so the hook IPC never stalls
command -v jq       >/dev/null 2>&1 || exit 0
command -v comemory >/dev/null 2>&1 || exit 0

# Shared canonical repo-scope key (basename of git-common-dir's parent), one
# definition for all three comemory entry points. Missing lib → silent no-op,
# consistent with this hook's non-fatal contract.
_rs="$(cd "${BASH_SOURCE%/*}/../lib" 2>/dev/null && pwd)/repo-scope.sh"
[ -r "$_rs" ] || exit 0
# shellcheck source=../lib/repo-scope.sh
. "$_rs"

if [ -n "${TOOLU_CONFIG_DIR:-}" ]; then
  CFG="$TOOLU_CONFIG_DIR"
elif [ "${TOOLU_HOST_OVERRIDE:-}" = codex ] || { [ -z "${TOOLU_HOST_OVERRIDE:-}" ] && [ -n "${PLUGIN_ROOT:-}" ]; }; then
  CFG="${CODEX_HOME:-${HOME:+$HOME/.codex}}"
else
  CFG="${CLAUDE_CONFIG_DIR:-${HOME:+$HOME/.claude}}"
fi
[ -n "$CFG" ] || exit 0
cwd=$(printf '%s' "$input" | jq -r '.cwd // empty' 2>/dev/null)
[ -n "$cwd" ] || cwd="${PWD:-}"
[ -n "$cwd" ] || exit 0

KEY=$(comemory_repo_key "$cwd")
[ -n "$KEY" ] || exit 0

# Bound the call if a timeout tool is present; otherwise run unbounded but still
# non-fatal (stock macOS has no `timeout`).
TO=""
command -v timeout  >/dev/null 2>&1 && TO="timeout 5"
command -v gtimeout >/dev/null 2>&1 && TO="gtimeout 5"
# comemory 0.9.0 changed `list --json` from a bare array to a paginated envelope
# {items,total,...}. Handle BOTH shapes (the plugin floor is 0.8.0): an array →
# its length; an object → its `.total` (the full count, not just this page).
count=$($TO comemory list --repo "$KEY" --json 2>/dev/null | jq -e '
  (if type=="array" then length else .total end)
  | select(type=="number" and . >= 0 and . == floor)
' 2>/dev/null) || exit 0

dir="$CFG/comemory-status"
mkdir -p "$dir" 2>/dev/null || exit 0
tmp=$(mktemp "$dir/.count.XXXXXX" 2>/dev/null) || exit 0
trap 'rm -f "$tmp"' EXIT
jq -nc --arg repo "$KEY" --argjson count "$count" \
  --arg updated "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  '{repo:$repo,count:$count,updated:$updated}' >"$tmp" 2>/dev/null \
  && mv -f "$tmp" "$dir/$KEY.json" 2>/dev/null

# Bootstrap nudge: an empty repo gets no benefit from the recall hooks above
# until something is indexed or saved. Point at the memory-bootstrap skill
# only when the count is exactly zero; otherwise stay silent as before.
if [ "$count" -eq 0 ] 2>/dev/null; then
  ctx="Comemory: $KEY has no memories yet. Load the memory-bootstrap skill to index code, index docs, distill past sessions, and save the decisions this repo cannot derive from itself."
  jq -nc --arg ctx "$ctx" '{hookSpecificOutput:{hookEventName:"SessionStart",additionalContext:$ctx}}'
fi
exit 0
