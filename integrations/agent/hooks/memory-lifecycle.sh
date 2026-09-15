#!/usr/bin/env bash
# Lightweight recall/compaction context and detached daily local maintenance.
set -euo pipefail
HOOK_DIR="$(cd "${BASH_SOURCE%/*}" && pwd)"
# shellcheck source=../lib/project-skills.sh
. "$HOOK_DIR/../lib/project-skills.sh"
input=$(cat)
command -v jq >/dev/null 2>&1 || exit 0
PS_CWD=$(jq -r '.cwd // empty' <<<"$input" 2>/dev/null) || exit 0
ps_memory_enabled || exit 0
event=$(jq -r '.hook_event_name // empty' <<<"$input")
case "$event" in
  UserPromptSubmit)
    prompt=$(jq -r '.prompt // empty' <<<"$input")
    case "$prompt" in ''|/comemory:*|\$comemory:*|ok|OK|thanks|Thanks) exit 0 ;; esac
    ctx="Recall relevant repo knowledge with $(ps_config_root)/comemory/comemory.sh search before exploration. Save verified reusable lessons promptly; avoid duplicate summaries."
    ;;
  PreCompact)
    ctx="Before compaction, persist verified reusable lessons through the repo-scoped comemory wrapper. Keep facts in memory and recurring procedures in project skills."
    ;;
  Stop)
    command -v comemory >/dev/null 2>&1 || exit 0
    root="$(ps_config_root)/comemory"
    mkdir -p "$root" || exit 0
    day=$(date -u +%Y%m%d)
    # Atomic mkdir avoids concurrent sessions launching duplicate maintenance.
    mkdir "$root/maintain-$day" 2>/dev/null || exit 0
    find "$root" -maxdepth 1 -type d -name 'maintain-*' -mtime +7 -empty -delete
    timeout_cmd=()
    if command -v timeout >/dev/null 2>&1; then timeout_cmd=(timeout 30)
    elif command -v gtimeout >/dev/null 2>&1; then timeout_cmd=(gtimeout 30); fi
    (
      for action in mine prune gc; do
        flags=()
        [ "$action" = gc ] || flags=(--apply)
        ${timeout_cmd[@]+"${timeout_cmd[@]}"} comemory "$action" ${flags[@]+"${flags[@]}"} || true
      done
    ) </dev/null >/dev/null 2>&1 &
    disown || true
    exit 0
    ;;
  *) exit 0 ;;
esac
jq -n --arg event "$event" --arg ctx "$ctx" '{hookSpecificOutput:{hookEventName:$event,additionalContext:$ctx}}'
