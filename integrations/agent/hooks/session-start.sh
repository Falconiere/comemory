#!/usr/bin/env bash
# Publish stable wrapper paths and provide the local-memory workflow.
set -euo pipefail
HOOK_DIR="$(cd "${BASH_SOURCE%/*}" && pwd)"
# shellcheck source=../lib/project-skills.sh
. "$HOOK_DIR/../lib/project-skills.sh"
input=$(cat)
command -v jq >/dev/null 2>&1 || exit 0
# shellcheck disable=SC2034 # Read by ps_load_cfg from the sourced library.
PS_CWD=$(jq -er '.cwd // empty' <<<"$input" 2>/dev/null) || PS_CWD="$PWD"
ps_memory_enabled || exit 0
root=$(ps_config_root)
mkdir -p "$root/comemory" || exit 0
for skill in agent-memory project-skills; do
  case "$skill" in agent-memory) file=comemory.sh ;; *) file=skills.sh ;; esac
  target="$root/comemory/$file"
  if [ -L "$target" ] || [ ! -e "$target" ]; then
    ln -sf "$HOOK_DIR/../skills/$skill/scripts/$file" "$target"
  fi
done

# Session-start marker: the Stop branch of memory-lifecycle.sh reads this as
# its `--since` bound for the shared-activity advisory. WRITE-ONCE: SessionStart also
# fires on resume/clear/compact with the SAME session_id, and rewriting the
# marker would move `--since` forward past unjudged recalls. Swept the way
# the Stop branch sweeps its own `maintain-*` directories.
session_id=$(jq -r '.session_id // empty' <<<"$input" 2>/dev/null) || session_id=""
# Sanitised: an untrusted session_id with '/' or '..' must not escape
# $root/comemory via the marker file name below.
session_id=$(ps_sanitize_session_id "$session_id") || session_id=""
if [ -n "$session_id" ]; then
  marker="$root/comemory/session-$session_id.start"
  [ -e "$marker" ] || printf '%s' "$(ps_now_precise)" >"$marker" 2>/dev/null || true
  find "$root/comemory" -maxdepth 1 -type f \
    \( -name 'session-*.start' -o -name 'session-*.blocked' -o -name 'session-*.advised' \) \
    ! -name "session-$session_id.start" ! -name "session-$session_id.blocked" \
    ! -name "session-$session_id.advised" -mtime +7 -delete 2>/dev/null || true
else
  find "$root/comemory" -maxdepth 1 -type f \
    \( -name 'session-*.start' -o -name 'session-*.blocked' -o -name 'session-*.advised' \) \
    -mtime +7 -delete 2>/dev/null || true
fi

ctx="Comemory: before exploration use MCP find (k=3), selectively show useful IDs, then feedback. If MCP is unavailable, use $root/comemory/comemory.sh. Save verified reusable lessons; keep recurring procedures in project skills."
jq -n --arg ctx "$ctx" '{hookSpecificOutput:{hookEventName:"SessionStart",additionalContext:$ctx}}'
