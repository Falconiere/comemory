#!/usr/bin/env bash
# comemory.sh — shared wrapper for the Claude Code plugin. Sole authority for
# git-repo scoping and missing-binary fail-soft; hooks and skills call only
# this, never `comemory` directly. Forwards the CLI's real exit code so skills
# see genuine failures (the SessionStart hook is what swallows non-zero).
#
# Usage: comemory.sh <save|list|context|search|search-code|...> [args...]
# `save` reads the memory body on stdin (positional "-"), so callers feed a
# quoted heredoc and multi-line bodies need no escaping.
set -uo pipefail

# Missing binary: emit a sentinel and exit 0 so a not-yet-installed comemory
# never breaks a session, hook, or skill. The sentinel is JSON regardless of
# whether the caller passed --json; callers substring-match
# `"comemory":"unavailable"` rather than parse, so this stays JSON in all modes.
if ! command -v comemory >/dev/null 2>&1; then
    printf '%s\n' '{"comemory":"unavailable","hint":"cargo install comemory"}'
    exit 0
fi

# Repo scope: COMEMORY_REPO override → git-root basename → "unknown".
repo="${COMEMORY_REPO:-}"
if [ -z "$repo" ]; then
    if root=$(git rev-parse --show-toplevel 2>/dev/null); then
        repo=$(basename "$root")
    else
        repo="unknown"
    fi
fi

sub="${1:-}"
[ "$#" -gt 0 ] && shift

# The injected `--repo "$repo"` precedes "$@", so a caller's explicit
# `--repo X` lands last and wins (clap's repeated-Option is last-wins).
case "$sub" in
    save)
        # Body on stdin via positional "-"; --repo scopes the memory.
        exec comemory save - --repo "$repo" "$@"
        ;;
    list | context | search | search-code)
        exec comemory "$sub" --repo "$repo" "$@"
        ;;
    -*)
        # Subcommand must come first. A leading global flag (e.g. `--data-dir`,
        # `--json`) would otherwise fall through to the unscoped `*)` arm and
        # silently skip `--repo` injection, filing a memory under the wrong
        # scope. Refuse it; callers (skills + hooks) always lead with the sub.
        printf '%s\n' \
            "error: subcommand must precede flags; got '$sub'" \
            'usage: comemory.sh <save|list|context|search|search-code> [args...]' >&2
        exit 64
        ;;
    "")
        printf '%s\n' \
            'usage: comemory.sh <save|list|context|search|search-code> [args...]' >&2
        exit 64
        ;;
    *)
        # Unknown subcommand: forward unscoped so the plugin never blocks a
        # valid comemory command it does not model.
        exec comemory "$sub" "$@"
        ;;
esac
