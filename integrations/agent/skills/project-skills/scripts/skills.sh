#!/usr/bin/env bash
# skills.sh — project-skill CLI for agent-created SKILL.md files under
# <repo>/.toolu/skills/. Repo-scoped via git toplevel. Does not require the
# comemory binary.
set -euo pipefail

_self="${BASH_SOURCE[0]}"
case "$_self" in */*) ;; *) _self="./$_self" ;; esac
_hops=0
while [ -L "$_self" ]; do
  if [ "$_hops" -ge 40 ]; then
    printf 'skills.sh: symlink chain exceeds 40 hops at %s\n' "$_self" >&2
    break
  fi
  _hops=$((_hops + 1))
  _link=$(readlink "$_self")
  case "$_link" in
    /*) _self="$_link" ;;
    *)  _self="${_self%/*}/$_link" ;;
  esac
done
_dir=$(cd "${_self%/*}" 2>/dev/null && pwd) || _dir="."
# scripts/ -> skill -> skills -> plugin root -> lib/project-skills.sh
_lib="$_dir/../../../lib/project-skills.sh"
if ! command -v jq >/dev/null 2>&1; then
  printf 'skills.sh: jq is required\n' >&2
  exit 1
fi
if [ ! -r "$_lib" ]; then
  printf 'skills.sh: lib not found at %s\n' "$_lib" >&2
  exit 1
fi
# shellcheck source=../../../lib/project-skills.sh
. "$_lib"

usage() {
  cat <<'USAGE'
Usage: skills.sh <subcommand> [args...]

  create <name> --description <text> [--file PATH]
  list [--json]
  index
  archive <name>
  restore <name>
  pin <name>
  unpin <name>
  adopt <name>
  status
  curate [--dry-run]

Project skills live at <repo>/.toolu/skills/<name>/SKILL.md.
USAGE
  exit 1
}

subcmd="${1:-}"
shift 2>/dev/null || true

case "$subcmd" in
  create)
    name="${1:-}"
    [ -n "$name" ] || usage
    shift
    desc="" src=""
    while [ $# -gt 0 ]; do
      case "$1" in
        --description)
          desc="${2:-}"
          [ -n "$desc" ] || { printf 'skills.sh: --description needs a value\n' >&2; exit 1; }
          shift 2
          ;;
        --file)
          src="${2:-}"
          [ -n "$src" ] || { printf 'skills.sh: --file needs a path\n' >&2; exit 1; }
          shift 2
          ;;
        *)
          printf 'skills.sh: unknown flag %s\n' "$1" >&2
          exit 1
          ;;
      esac
    done
    ps_create "$name" "$desc" "$src"
    ;;
  list)
    json=0
    [ "${1:-}" = "--json" ] && json=1
    ps_list "$json"
    ;;
  index)
    ps_index
    ;;
  archive)
    [ -n "${1:-}" ] || usage
    ps_archive "$1"
    ;;
  restore)
    [ -n "${1:-}" ] || usage
    ps_restore "$1"
    ;;
  pin)
    [ -n "${1:-}" ] || usage
    ps_set_pin "$1" true
    ;;
  unpin)
    [ -n "${1:-}" ] || usage
    ps_set_pin "$1" false
    ;;
  adopt)
    [ -n "${1:-}" ] || usage
    ps_adopt "$1"
    ;;
  status)
    ps_status
    ;;
  curate)
    dry=0
    [ "${1:-}" = "--dry-run" ] && dry=1
    ps_curate "$dry"
    ;;
  ""|-h|--help|help)
    usage
    ;;
  *)
    printf 'skills.sh: unknown subcommand %s\n' "$subcmd" >&2
    usage
    ;;
esac
