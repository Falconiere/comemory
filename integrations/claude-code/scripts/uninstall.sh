#!/usr/bin/env bash
# uninstall.sh — back the comemory Claude Code plugin out for an end user.
# Prints how to unhook the plugin from Claude Code and, by default, LEAVES your
# comemory memories (the data dir) untouched. Pass --purge-data to also delete
# them, behind a typed confirmation. Fail-soft; never deletes without consent.
set -uo pipefail

purge=0
case "${1:-}" in
    --purge-data) purge=1 ;;
    "") ;;
    *) printf 'usage: uninstall.sh [--purge-data]\n' >&2; exit 64 ;;
esac

cat <<'EOF'
Remove the plugin from Claude Code:
  - In Claude Code, run `/plugin` and uninstall "comemory", or
  - remove its entry from the plugins/marketplace section of ~/.claude/settings.json.

That unhooks SessionStart auto-recall and the comemory-* skills. The `comemory`
binary and your stored memories are NOT affected by that step.
EOF

data="${COMEMORY_DATA_DIR:-$HOME/.comemory}"

if [ "$purge" -eq 1 ]; then
    if [ -d "$data" ]; then
        printf '\nThis PERMANENTLY deletes all comemory data at:\n  %s\n' "$data"
        printf 'Type that exact path to confirm: '
        read -r reply
        if [ "$reply" = "$data" ]; then
            rm -rf -- "$data" && printf 'Removed %s\n' "$data"
        else
            printf 'Confirmation did not match — left %s intact.\n' "$data"
        fi
    else
        printf '\nNo data dir at %s — nothing to purge.\n' "$data"
    fi
elif [ -d "$data" ]; then
    printf '\nYour memories remain at: %s\n' "$data"
    printf 'Re-run with --purge-data to delete them too.\n'
fi
