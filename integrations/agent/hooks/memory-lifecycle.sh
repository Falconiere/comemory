#!/usr/bin/env bash
# Recall injection, compaction context, Stop-time recall enforcement, and
# detached daily local maintenance. Helpers for injection/enforcement live in
# ../lib/recall.sh (split out once this hook grew past ~120 lines).
set -euo pipefail
HOOK_DIR="$(cd "${BASH_SOURCE%/*}" && pwd)"
# shellcheck source=../lib/project-skills.sh
. "$HOOK_DIR/../lib/project-skills.sh"
# shellcheck source=../lib/recall.sh
. "$HOOK_DIR/../lib/recall.sh"
input=$(cat)
command -v jq >/dev/null 2>&1 || exit 0
PS_CWD=$(jq -r '.cwd // empty' <<<"$input" 2>/dev/null) || exit 0
ps_memory_enabled || exit 0
event=$(jq -r '.hook_event_name // empty' <<<"$input")
case "$event" in
  UserPromptSubmit)
    prompt=$(jq -r '.prompt // empty' <<<"$input")
    case "$prompt" in ''|/comemory:*|\$comemory:*|ok|OK|thanks|Thanks) exit 0 ;; esac
    # comemory absent → nothing to recall from and no reminder about it either
    # (mirrors the Stop branch's own comemory guard below).
    command -v comemory >/dev/null 2>&1 || exit 0
    ctx="Recall relevant repo knowledge with $(ps_config_root)/comemory/comemory.sh search before exploration. Save verified reusable lessons promptly; avoid duplicate summaries."
    # Character floor: `${#prompt}` counts CHARACTERS under a multibyte-aware
    # LC_CTYPE and BYTES under C/POSIX, so a prompt with non-ASCII text reads
    # longer than it is and can clear the floor early under C. Scope a UTF-8
    # LC_CTYPE to this one measurement when a UTF-8 locale is installed;
    # otherwise fall back to the ambient count — the prompt is untrusted free
    # text, not a path, so the worst case here is the threshold landing a
    # few characters off, not a correctness or safety bug.
    prompt_len="${#prompt}"
    if locale -a 2>/dev/null | grep -Eqi '^en_US\.utf-?8$'; then
      # LC_ALL, not LC_CTYPE: an ambient LC_ALL would outrank a bare LC_CTYPE.
      prompt_len=$(LC_ALL=en_US.UTF-8; printf '%s' "${#prompt}")
    fi
    if [ "$(ps_cfg_bool recall.inject true)" = true ] && [ "$prompt_len" -ge "$(ps_cfg_int recall.injectMinChars 24)" ]; then
      repo=$(comemory_repo_key "$PS_CWD")
      if [ -n "$repo" ]; then
        k=$(ps_cfg_int recall.injectK 3)
        raw=$(recall_find_json "$repo" "$k" "$prompt") || raw=""
        hint=$(recall_hint_from_json "$raw") || hint=""
        [ -n "$hint" ] && ctx="$hint"
      fi
    fi
    ;;
  PreCompact)
    ctx="Before compaction, persist verified reusable lessons through the repo-scoped comemory wrapper. Keep facts in memory and recurring procedures in project skills."
    ;;
  Stop)
    # Enforcement runs BEFORE the once-per-day maintenance latch, in its own
    # function; when it blocks it prints the decision and exits without
    # touching the latch below.
    recall_enforce_block "$input" "$PS_CWD" && exit 0
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
