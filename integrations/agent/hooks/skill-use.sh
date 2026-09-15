#!/usr/bin/env bash
# PostToolUse — increment use_count on a Read of an on-disk project skill,
# patch_count on a Write/Edit. Never blocks. Matcher names (hooks.json):
#   read:  Read (Claude), read_file (Codex/Grok)
#   write: Edit, Write, MultiEdit, apply_patch, search_replace, write
set -u

HOOK_DIR="$(cd "$(dirname "$0")" 2>/dev/null && pwd)" || HOOK_DIR="."
# shellcheck source=../lib/project-skills.sh
. "$HOOK_DIR/../lib/project-skills.sh"

input=$(cat 2>/dev/null || echo '{}')
command -v jq >/dev/null 2>&1 || exit 0

tool_name=$(jq -r '.tool_name // ""' <<<"$input" 2>/dev/null || echo "")
kind=""
case "$tool_name" in
  Read|read_file) kind="use" ;;
  Edit|Write|MultiEdit|apply_patch|search_replace|write) kind="patch" ;;
  *) exit 0 ;;
esac

cwd=$(jq -r '.cwd // empty' <<<"$input" 2>/dev/null || true)
[ -n "$cwd" ] || cwd="${PWD:-.}"
PS_CWD="$cwd"
ps_loop_enabled || exit 0

rel=$(jq -r '
  .tool_input.file_path
  // .tool_input.target_file
  // .tool_input.path
  // empty
' <<<"$input" 2>/dev/null || true)
[ -n "$rel" ] || exit 0

case "$rel" in
  /*) abs="$rel" ;;
  *)  abs="$cwd/$rel" ;;
esac

# Resolve .. and the file itself (SKILL.md may be a symlink into .archive/).
abs=$(ps_realpath "$abs") || exit 0

# Reject archive paths and anything that is not .../.toolu/skills/<name>/SKILL.md
name=""
case "$abs" in
  */.toolu/skills/.archive/*) exit 0 ;;
  */.toolu/skills/*/SKILL.md)
    rest="${abs#*/.toolu/skills/}"
    name="${rest%%/SKILL.md}"
    case "$name" in
      */*|.*|"") exit 0 ;;
    esac
    ;;
  *) exit 0 ;;
esac
[ -n "$name" ] || exit 0

ps_record "$kind" "$name"
exit 0
