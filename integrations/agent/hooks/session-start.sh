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
ctx="Comemory: use $root/comemory/comemory.sh for repo-scoped recall before exploration. Save verified corrections, decisions and fixes with evidence; compare existing memories and supersede outdated ones. Record feedback when recall helped. Procedures belong in project skills; patch an existing skill first."
jq -n --arg ctx "$ctx" '{hookSpecificOutput:{hookEventName:"SessionStart",additionalContext:$ctx}}'
