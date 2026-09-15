#!/usr/bin/env bash
# SessionStart — inject a compact project-skills catalog (name + description).
# Empty tree: emit nothing. Never nags.
set -u

HOOK_DIR="$(cd "$(dirname "$0")" 2>/dev/null && pwd)" || HOOK_DIR="."
# shellcheck source=../lib/project-skills.sh
. "$HOOK_DIR/../lib/project-skills.sh"

input=$(cat 2>/dev/null || echo '{}')
command -v jq >/dev/null 2>&1 || exit 0

cwd=$(jq -r '.cwd // empty' <<<"$input" 2>/dev/null || true)
[ -n "$cwd" ] || cwd="${PWD:-.}"
PS_CWD="$cwd"
ps_loop_enabled || exit 0

root=$(ps_repo_root "$cwd")
[ -n "$root" ] || exit 0
[ -d "$(ps_skills_dir "$root")" ] || exit 0

index=$(PS_CWD="$root" ps_index)
[ -n "$index" ] || exit 0

header="Project skills — Read .toolu/skills/<name>/SKILL.md to load the procedure. Do not rely on this index as the body."
ctx=$(printf '%s\n%s\n' "$header" "$index")
jq -n --arg ctx "$ctx" '{
  hookSpecificOutput: {
    hookEventName: "SessionStart",
    additionalContext: $ctx
  }
}'
exit 0
