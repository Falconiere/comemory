#!/usr/bin/env bash
# Publish stable wrapper paths and provide the local-memory workflow.
set -euo pipefail
HOOK_DIR="$(cd "${BASH_SOURCE%/*}" && pwd)"
# shellcheck source=../lib/project-skills.sh
. "$HOOK_DIR/../lib/project-skills.sh"
input=$(cat)
command -v jq >/dev/null 2>&1 || exit 0
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
# its `--since` bound for recall enforcement. WRITE-ONCE: SessionStart also
# fires on resume/clear/compact with the SAME session_id, and rewriting the
# marker would move `--since` forward past unjudged recalls. Swept the way
# the Stop branch sweeps its own `maintain-*` directories.
session_id=$(jq -r '.session_id // empty' <<<"$input" 2>/dev/null) || session_id=""
# Sanitised: an untrusted session_id with '/' or '..' must not escape
# $root/comemory via the marker file name below.
session_id=$(ps_sanitize_session_id "$session_id") || session_id=""
if [ -n "$session_id" ]; then
  marker="$root/comemory/session-$session_id.start"
  [ -e "$marker" ] || printf '%s' "$(ps_now)" >"$marker" 2>/dev/null || true
fi
find "$root/comemory" -maxdepth 1 -type f -name 'session-*.start' -mtime +7 -delete 2>/dev/null || true

ctx="Comemory: use $root/comemory/comemory.sh for repo-scoped recall before exploration. Save verified corrections, decisions and fixes with evidence; compare existing memories and supersede outdated ones. Record feedback when recall helped. Procedures belong in project skills; patch an existing skill first."
jq -n --arg ctx "$ctx" '{hookSpecificOutput:{hookEventName:"SessionStart",additionalContext:$ctx}}'
