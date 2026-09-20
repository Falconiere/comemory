#!/usr/bin/env bash
# SessionEnd hook — capture a session receipt without delaying the host's
# SessionEnd budget (~1.5s).
#
# `comemory capture session --from-hook` reads the SessionEnd payload
# (session_id / transcript_path) from its own stdin. Launched DETACHED
# (background subshell + disown) because "not logged in" or a slow platform
# round trip must never delay Stop/SessionEnd — `--from-hook` already fails
# quietly on those conditions, and this hook never waits to find out. The
# whole subshell is redirected from/to /dev/null so the child inherits none
# of the hook's own stdio (a child holding the hook's stdout open would hang
# any `$( … )` capture of this script), and the payload is re-fed from the
# $input variable via a here-string INSIDE the subshell rather than the
# hook's own stdin, which is already closed by that redirection.
set -euo pipefail
HOOK_DIR="$(cd "${BASH_SOURCE%/*}" && pwd)"
# shellcheck source=../lib/project-skills.sh
. "$HOOK_DIR/../lib/project-skills.sh"
input=$(cat)
# Capture currently accepts Claude Code JSONL transcripts only. Codex installs
# this shared bundle too, but must leave SessionEnd silent until its transcript
# format has a supported capture path.
if [ "${TOOLU_HOST_OVERRIDE:-}" = codex ] \
  || { [ -z "${TOOLU_HOST_OVERRIDE:-}" ] && [ -n "${PLUGIN_ROOT:-}" ]; }; then
  exit 0
fi
command -v jq >/dev/null 2>&1 || exit 0
# shellcheck disable=SC2034 # Read by ps_load_cfg from the sourced library.
PS_CWD=$(jq -r '.cwd // empty' <<<"$input" 2>/dev/null) || exit 0
ps_memory_enabled || exit 0
command -v comemory >/dev/null 2>&1 || exit 0
timeout_cmd=()
if command -v timeout >/dev/null 2>&1; then timeout_cmd=(timeout 30)
elif command -v gtimeout >/dev/null 2>&1; then timeout_cmd=(gtimeout 30); fi
(
  ${timeout_cmd[@]+"${timeout_cmd[@]}"} comemory capture session --from-hook <<<"$input"
) </dev/null >/dev/null 2>&1 &
disown || true
exit 0
