#!/usr/bin/env bash
# scripts/validate-release.sh — preflight checks before tagging a release.
#
# Hard checks (exit 1 on any failure):
#   1. Working tree is clean (no staged or unstaged changes; untracked
#      files are tolerated — see `git status --porcelain
#      --untracked-files=no` below).
#   2. Current branch is `main` (override with $RELEASE_BRANCH).
#   3. Cargo.toml `version` matches the requested version.
#   4. CHANGELOG.md has a `## [<version>] - YYYY-MM-DD` heading dated today.
#
# Soft warnings (exit 0, but print a yellow warning line):
#   - Cargo.lock is dirty.
#   - git user.email is not configured.
#   - The latest CI run on the tip commit is not 'success' (skipped if `gh`
#     is not on PATH or the repo has no GitHub remote).
#
# Usage: bash scripts/validate-release.sh <version>
#   <version> is the bare semver (e.g. 0.11.0 or 0.11.0-rc.1), no leading 'v'.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

step="validate-release"
ver="${1:-}"
if [[ -z "$ver" ]]; then
  die "$step" "usage: bash $0 <version>  (e.g. 0.11.0 or 0.11.0-rc.1)"
fi

branch="${RELEASE_BRANCH:-main}"
today="$(date -u +%Y-%m-%d)"
fails=0
warns=0

check() {
  local name="$1" ok="$2" detail="$3"
  if (( ok )); then
    log_ok "$step" "[ok]   $name — $detail"
  else
    log_err "$step" "[fail] $name — $detail"
    fails=$((fails + 1))
  fi
}

warn() {
  local name="$1" detail="$2"
  printf '%s[%s]%s %s[warn]%s %s — %s\n' \
    "$C_DIM" "$step" "$C_RST" "$C_YLW" "$C_RST" "$name" "$detail" >&2
  warns=$((warns + 1))
}

cd "$PROJECT_ROOT"

# 1. Working tree clean (allow untracked dotfiles / build artifacts).
# We accept untracked files but require no staged/unstaged modifications.
dirty="$(git status --porcelain --untracked-files=no || true)"
check "working-tree-clean" \
  "$([[ -z "$dirty" ]] && echo 1 || echo 0)" \
  "$([[ -z "$dirty" ]] && echo "no modified or staged files" \
    || echo "modified files: $(echo "$dirty" | wc -l | tr -d ' ')")"

# 2. Current branch is main (or $RELEASE_BRANCH).
current_branch="$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo unknown)"
check "on-release-branch" \
  "$([[ "$current_branch" == "$branch" ]] && echo 1 || echo 0)" \
  "current: $current_branch (expected: $branch)"

# 3. Cargo.toml version matches.
cargo_ver="$(grep -E '^version[[:space:]]*=' Cargo.toml | head -1 \
  | sed -E 's/^version[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/')"
if [[ -z "$cargo_ver" ]]; then
  cargo_ver="(not found)"
fi
check "cargo-toml-version" \
  "$([[ "$cargo_ver" == "$ver" ]] && echo 1 || echo 0)" \
  "Cargo.toml says '$cargo_ver', expected '$ver'"

# 4. CHANGELOG.md has a `## [<version>] — YYYY-MM-DD` heading dated today.
changelog_heading_pattern="^## \[${ver}\] - ${today}\$"
if grep -qE "$changelog_heading_pattern" CHANGELOG.md; then
  check "changelog-heading" 1 "## [$ver] - $today present"
else
  # Check what's actually there for a useful error message.
  actual="$(grep -E "^## \[${ver}\]" CHANGELOG.md 2>/dev/null || echo "(no matching heading)")"
  check "changelog-heading" 0 \
    "expected '## [$ver] - $today', found: $actual"
fi

# Soft warnings.
# Cargo.lock dirty (uncommitted).
lock_dirty="$(git diff --name-only -- Cargo.lock 2>/dev/null || true)"
if [[ -n "$lock_dirty" ]]; then
  warn "cargo-lock-dirty" "Cargo.lock has uncommitted changes"
fi

# git user.email not configured.
user_email="$(git config user.email 2>/dev/null || true)"
if [[ -z "$user_email" ]]; then
  warn "git-user-email" "git config user.email is empty (commits will use the system default)"
fi

# CI status on the tip commit (best-effort; skip if `gh` is absent).
if command -v gh >/dev/null 2>&1; then
  remote="$(git remote get-url origin 2>/dev/null || true)"
  if [[ -n "$remote" ]]; then
    # `gh` requires repo in owner/name form; derive from origin URL.
    if [[ "$remote" =~ github\.com[:/]([^/]+)/([^/.]+) ]]; then
      gh_repo="${BASH_REMATCH[1]}/${BASH_REMATCH[2]}"
      tip_sha="$(git rev-parse HEAD)"
      ci_status="$(gh run list --repo "$gh_repo" --commit "$tip_sha" \
        --json status --jq '.[0].status // "no-runs"' 2>/dev/null || echo unknown)"
      if [[ "$ci_status" != "success" && "$ci_status" != "no-runs" && "$ci_status" != "unknown" ]]; then
        warn "ci-status" "latest run on $tip_sha: $ci_status (expected: success)"
      fi
    fi
  fi
fi

# Summary.
if (( fails > 0 )); then
  log_err "$step" "$fails hard check(s) failed; aborting"
  exit 1
fi
log_ok "$step" "all hard checks passed ($warns warning(s))"
