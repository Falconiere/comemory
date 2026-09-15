#!/usr/bin/env bash
# Stop hook — once-per-UTC-day detached curator for agent-created project skills.
# Same timeout/disown pattern as plugins/toolu/hooks/session-end.sh so a hung
# run can never delay Stop. Stamp is written BEFORE the work.
set -u

HOOK_DIR="$(cd "$(dirname "$0")" 2>/dev/null && pwd)" || HOOK_DIR="."
# shellcheck source=../lib/project-skills.sh
. "$HOOK_DIR/../lib/project-skills.sh"

# Drain stdin so the host's hook IPC does not stall.
input=$(cat)
command -v jq >/dev/null 2>&1 || exit 0
PS_CWD=$(jq -r '.cwd // empty' <<<"$input" 2>/dev/null) || exit 0

ps_loop_enabled || exit 0

_cm_stamp_dir="$(ps_config_root)/comemory"
_cm_stamp="$_cm_stamp_dir/.project-skills-last-curate"
_cm_today="$(date -u +%Y%m%d 2>/dev/null || echo '')"
[ -n "$_cm_today" ] || exit 0
if [ "$(cat "$_cm_stamp" 2>/dev/null || echo '')" = "$_cm_today" ]; then
  exit 0
fi
mkdir -p "$_cm_stamp_dir" 2>/dev/null || true
printf '%s' "$_cm_today" >"$_cm_stamp" 2>/dev/null || true

_sh=""
if [ -n "${TOOLU_PROJECT_SKILLS_SH:-}" ] && [ -x "${TOOLU_PROJECT_SKILLS_SH:-}" ]; then
  _sh="$TOOLU_PROJECT_SKILLS_SH"
elif command -v skills.sh >/dev/null 2>&1; then
  _sh="skills.sh"
else
  _cand="$HOOK_DIR/../skills/project-skills/scripts/skills.sh"
  [ -x "$_cand" ] && _sh="$_cand"
fi
[ -n "$_sh" ] || exit 0

_cmd=()
if command -v nohup >/dev/null 2>&1; then _cmd+=(nohup); fi
if command -v timeout >/dev/null 2>&1; then _cmd+=(timeout 30)
elif command -v gtimeout >/dev/null 2>&1; then _cmd+=(gtimeout 30); fi
_cmd+=("$_sh" curate)
"${_cmd[@]}" </dev/null >/dev/null 2>&1 &
disown 2>/dev/null || true
exit 0
