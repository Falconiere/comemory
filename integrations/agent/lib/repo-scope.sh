#!/usr/bin/env bash
# shellcheck shell=bash
# Shared repo-scope helpers for the comemory plugin.
#
# The canonical memory/index scope is the basename of the parent of
# git-common-dir — STABLE across every git worktree (all worktrees share one
# .git, and --git-common-dir points at the main worktree's .git from anywhere),
# so saves made in a worktree stay visible from main and sibling worktrees. A
# bare --show-toplevel would mint a per-worktree scope and split the store.
# Falls back to --show-toplevel basename for custom GIT_DIR / bare layouts, then
# empty outside any repo. Sourced by comemory-status.sh, setup.sh, and the
# agent-memory comemory.sh wrapper so all three derive ONE key (previously three
# hand-mirrored copies that had drifted — repo_key lacked the --show-toplevel
# fallback the other two had).

# comemory_repo_key [dir] — echo the canonical repo key (basename), or empty.
# $1 optional working dir (defaults to "."); callers that run from elsewhere
# (the SessionStart badge hook) pass an explicit dir. Always returns 0 so a
# `set -e` caller's `REPO=$(comemory_repo_key)` reaches its own empty fallback
# instead of aborting outside a git repo.
comemory_repo_key() {
  local dir="${1:-.}" common top
  # --path-format=absolute needs git >= 2.31; retry plain then absolutize for older git.
  common=$(git -C "$dir" rev-parse --path-format=absolute --git-common-dir 2>/dev/null || true)
  if [ -z "$common" ]; then
    common=$(git -C "$dir" rev-parse --git-common-dir 2>/dev/null || true)
    [ -n "$common" ] && common=$(cd "$dir" 2>/dev/null && cd "$common" 2>/dev/null && pwd || true)
  fi
  case "$common" in
    */.git) basename "$(dirname "$common")" ;;
    *)
      top=$(git -C "$dir" rev-parse --show-toplevel 2>/dev/null || true)
      if [ -n "$top" ]; then basename "$top"; fi
      ;;
  esac
}

# version_ge A B — return 0 if A >= B by version order (sort -V), else 1.
version_ge() { [ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -n1)" = "$2" ]; }
