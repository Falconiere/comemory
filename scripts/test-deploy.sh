#!/usr/bin/env bash
# scripts/test-deploy.sh — real-data self-check for scripts/deploy.sh.
#
# Every case that touches a repository runs against a fresh `git clone` of this
# checkout whose `origin` is this local repository, so no case can push to
# GitHub. Uncommitted work never reaches the clone.
#
# Usage: bash scripts/test-deploy.sh [case...]
#   args preflight bump formula push skill   fast, offline (the default set)
#   rehearsal   real three-target build through `--dry-run` (Docker, slow)
#   smoke       installs the already-published v0.57.1 release (network)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEPLOY="$ROOT/scripts/deploy.sh"
FIXTURE="$ROOT/tests/common/fixtures/homebrew/comemory-0.57.1.rb"
TARGETS=(aarch64-apple-darwin aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu)
WORK="$(mktemp -d "${TMPDIR:-/tmp}/test-deploy.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
# This machine's GH_TOKEN can be stale while the keyring login works.
if [[ -n "${GH_TOKEN:-}" ]] && ! gh api user >/dev/null 2>&1; then unset GH_TOKEN; fi
CASE=""

fail() { printf 'FAIL %s: %s\n' "$CASE" "$1" >&2; exit 1; }

# fresh_clone <name> — a clone of this checkout with no git hooks; prints its path.
fresh_clone() {
  local dir="$WORK/$1"
  git clone --quiet "$ROOT" "$dir"
  mkdir -p "$WORK/no-hooks"
  git -C "$dir" config core.hooksPath "$WORK/no-hooks"
  git -C "$dir" config user.name deploy-test
  git -C "$dir" config user.email deploy-test@example.invalid
  git -C "$dir" config commit.gpgsign false
  git -C "$dir" config tag.gpgsign false
  printf '%s\n' "$dir"
}

# version_of <dir> — the [package] version in <dir>/Cargo.toml.
version_of() { awk -F'"' '/^version[[:space:]]*=/ { print $2; exit }' "$1/Cargo.toml"; }

# bumped <version> <part> — next minor (part=minor) or patch (part=patch).
bumped() {
  local maj min pat
  IFS=. read -r maj min pat <<<"$1"
  if [[ "$2" == minor ]]; then printf '%s.%s.0\n' "$maj" "$((min + 1))"
  else printf '%s.%s.%s\n' "$maj" "$min" "$((pat + 1))"; fi
}

# state <dir> — branch tips, tags and tracked-file status: what a refusal must not change.
state() {
  git -C "$1" for-each-ref --format='%(refname) %(objectname)' refs/heads refs/tags
  git -C "$1" status --porcelain
}

# expect_refusal <dir> <exit> <message> <deploy args...>
expect_refusal() {
  local dir="$1" code="$2" msg="$3" before after err rc=0
  shift 3
  before="$(state "$dir")"
  err="$(cd "$dir" && bash "$DEPLOY" "$@" 2>&1 >/dev/null)" || rc=$?
  after="$(state "$dir")"
  [[ $rc -eq $code ]] || fail "deploy $* exited $rc, expected $code: $err"
  [[ "$err" == *"$msg"* ]] || fail "deploy $* said '$err', expected '$msg'"
  [[ "$before" == "$after" ]] || fail "deploy $* changed refs or tracked files"
}

# in_lib <dir> <code> — run <code> with the deploy libraries loaded, cwd = <dir>.
in_lib() {
  (cd "$1" && bash -c 'set -euo pipefail
    source "$0/lib/common.sh"
    for f in "$0"/lib/deploy/*.sh; do source "$f"; done
    eval "$1"' "$ROOT/scripts" "$2")
}

worktrees() { git -C "$1" worktree list --porcelain | grep -c '^worktree '; }

case_args() {
  local c cur next
  c="$(fresh_clone args)"; cur="$(version_of "$c")"; next="$(bumped "$cur" minor)"
  git -C "$c" tag -a -m existing "v$next"
  expect_refusal "$c" 64 "unknown argument: --env" --env prod --tag "v$next" --change-log x
  expect_refusal "$c" 64 "--tag needs a value" --tag
  expect_refusal "$c" 1 "--tag needs a stable vX.Y.Z" --change-log x
  expect_refusal "$c" 1 "--tag needs a stable vX.Y.Z" --tag "v$next-rc.1" --change-log x
  expect_refusal "$c" 1 "--change-log is empty" --tag "v$(bumped "$next" patch)" --change-log '  '
  expect_refusal "$c" 1 "v$cur is not newer than Cargo.toml's $cur" --tag "v$cur" --change-log x
  expect_refusal "$c" 1 "tag v$next already exists" --tag "v$next" --change-log x
}

case_preflight() {
  local c next
  c="$(fresh_clone preflight)"; next="$(bumped "$(version_of "$c")" minor)"
  git -C "$c" checkout --quiet -b feature
  expect_refusal "$c" 1 "releases come from main" --tag "v$next" --change-log x
  git -C "$c" checkout --quiet main
  printf '\n' >>"$c/README.md"
  expect_refusal "$c" 1 "tracked files are modified" --tag "v$next" --change-log x
  git -C "$c" checkout --quiet -- README.md
  git -C "$c" commit --quiet --allow-empty -m "local only"
  expect_refusal "$c" 1 "main is not equal to origin/main" --tag "v$next" --change-log x
  awk '{ print } /^## \[Unreleased\]/ { print ""; print "- pending entry" }' "$c/CHANGELOG.md" >"$WORK/cl"
  mv "$WORK/cl" "$c/CHANGELOG.md"
  git -C "$c" commit --quiet -am "unreleased entry"
  expect_refusal "$c" 1 "[Unreleased] has entries" --tag "v$next" --change-log x
}

case_bump() {
  local c cur next head commit day added want hooks
  c="$(fresh_clone bump)"; cur="$(version_of "$c")"; next="$(bumped "$cur" minor)"
  head="$(git -C "$c" rev-parse HEAD)"; day="$(date -u +%F)"
  commit="$(DEPLOY_TAG="v$next" DEPLOY_NOTES=$'### Added\n\n- a thing\n' \
    in_lib "$c" 'trap deploy_cleanup EXIT; deploy_validate_args
      deploy_prepare_bump >&2; printf "%s\n" "$DEPLOY_COMMIT"')" || fail "bump failed"
  [[ "$(git -C "$c" rev-parse HEAD)" == "$head" ]] || fail "caller branch moved"
  [[ "$(worktrees "$c")" -eq 1 ]] || fail "temp worktree left behind"
  [[ "$(git -C "$c" show "$commit:Cargo.toml" | awk -F'"' '/^version/ { print $2; exit }')" \
    == "$next" ]] || fail "Cargo.toml not bumped"
  git -C "$c" show "$commit:Cargo.lock" | awk '/^name = "comemory"$/ { getline; print }' \
    | grep -qx "version = \"$next\"" || fail "Cargo.lock not bumped"
  [[ "$(git -C "$c" show "$commit:CHANGELOG.md" | awk '/^## \[/ { if (++n == 2) { print; exit } }')" \
    == "## [$next] - $day" ]] || fail "new section is not first under [Unreleased]"
  git -C "$c" diff --unified=0 "$head" "$commit" -- CHANGELOG.md | grep -q '^-[^-]' \
    && fail "existing CHANGELOG lines changed"
  added="$(git -C "$c" diff --unified=0 "$head" "$commit" -- CHANGELOG.md \
    | sed -n 's/^+//p' | grep -v '^++ ' | sort)"
  want="$(printf '\n## [%s] - %s\n\n### Added\n\n- a thing\n' "$next" "$day" | sort)"
  [[ "$added" == "$want" ]] || fail "CHANGELOG insertion differs: $added"
  hooks="$WORK/failing-hooks"; mkdir -p "$hooks"
  printf '#!/bin/sh\nexit 1\n' >"$hooks/pre-commit"; chmod +x "$hooks/pre-commit"
  git -C "$c" config core.hooksPath "$hooks"
  if DEPLOY_TAG="v$(bumped "$next" patch)" DEPLOY_NOTES=x in_lib "$c" \
    'trap deploy_cleanup EXIT; deploy_validate_args; deploy_prepare_bump' 2>/dev/null; then
    fail "bump succeeded despite a refusing pre-commit hook"
  fi
  [[ "$(worktrees "$c")" -eq 1 ]] || fail "temp worktree left after a refused commit"
  [[ "$(git -C "$c" rev-parse HEAD)" == "$head" ]] || fail "caller branch moved after refusal"
}

case_formula() {
  local sums="$WORK/sha256.sum" out t sum
  for t in "${TARGETS[@]}"; do
    printf '%s *comemory-%s.tar.xz\n' "$(printf '%s' "$t" | shasum -a 256 | cut -d' ' -f1)" "$t"
  done >"$sums"
  out="$(in_lib "$ROOT" "deploy_rewrite_formula '$FIXTURE' 0.58.0 '$sums'")" || fail "rewrite failed"
  grep -qx '  version "0.58.0"' <<<"$out" || fail "version line not rewritten"
  [[ "$(grep -c '/releases/download/v0.58.0/comemory-' <<<"$out")" -eq 3 ]] || fail "urls not rewritten"
  for t in "${TARGETS[@]}"; do
    sum="$(awk -v t="comemory-$t.tar.xz" '$2 == "*" t { print $1 }' "$sums")"
    awk -v t="/comemory-$t.tar.xz\"" 'index($0, t) { getline; print; exit }' <<<"$out" \
      | grep -q "sha256 \"$sum\"" || fail "$t sha256 not its own checksum"
  done
  [[ "$(diff "$FIXTURE" - <<<"$out" | grep -c '^>')" -eq 7 ]] || fail "rewrite touched other lines"
  if in_lib "$ROOT" "deploy_rewrite_formula '$FIXTURE' 0.58.0 /dev/null" >/dev/null 2>&1; then
    fail "rewrite accepted a sum file without checksums"
  fi
}

case_push() {
  local c bare other cur next later commit
  c="$(fresh_clone push)"; bare="$WORK/remote.git"
  git -C "$c" checkout --quiet main
  git clone --quiet --bare "$c" "$bare"
  git -C "$c" remote set-url origin "$bare"; git -C "$c" fetch --quiet origin
  cur="$(version_of "$c")"; next="$(bumped "$cur" minor)"; later="$(bumped "$next" patch)"
  commit="$(DEPLOY_TAG="v$next" DEPLOY_NOTES=x in_lib "$c" \
    'trap deploy_cleanup EXIT; deploy_validate_args; deploy_prepare_bump >&2
     deploy_push >&2; printf "%s\n" "$DEPLOY_COMMIT"')" || fail "push failed"
  [[ "$(git -C "$bare" rev-parse main)" == "$commit" ]] || fail "remote main is not the release commit"
  [[ "$(git -C "$bare" rev-parse "v$next^{commit}")" == "$commit" ]] || fail "remote tag missing"
  [[ "$(git -C "$c" rev-parse main)" == "$commit" ]] || fail "local main not fast-forwarded"
  other="$WORK/other"; git clone --quiet "$bare" "$other"
  git -C "$other" -c user.name=x -c user.email=x@example.invalid -c commit.gpgsign=false commit --quiet --allow-empty -m moved
  git -C "$other" push --quiet origin main
  if DEPLOY_TAG="v$later" DEPLOY_NOTES=x in_lib "$c" \
    'trap deploy_cleanup EXIT; deploy_validate_args; deploy_prepare_bump; deploy_push' 2>"$WORK/err"; then
    fail "push succeeded although remote main moved"
  fi
  grep -q "push rejected" "$WORK/err" || fail "rejection not named: $(cat "$WORK/err")"
  git -C "$c" rev-parse -q --verify "refs/tags/v$later" >/dev/null && fail "local tag kept after rejection"
  [[ "$(git -C "$c" rev-parse main)" == "$commit" ]] || fail "local main moved after rejection"
  git -C "$bare" rev-parse -q --verify "refs/tags/v$later" >/dev/null && fail "remote got the tag"
  return 0
}

case_skill() {
  local f="$ROOT/.toolu/skills/manual-deploy/SKILL.md" s
  [[ -f "$f" ]] || fail "missing $f"
  grep -qx 'name: manual-deploy' "$f" || fail "frontmatter name"
  grep -q '^description: Use when' "$f" || fail "description must start with 'Use when'"
  for s in '## When to Use' '## Procedure' '## Pitfalls' '## Verification'; do
    grep -qx "$s" "$f" || fail "missing section $s"
  done
  for s in 'scripts/deploy.sh --tag' '--dry-run' 'env -u GH_TOKEN' 'git log'; do
    grep -qF -- "$s" "$f" || fail "skill does not mention '$s'"
  done
}

case_smoke() {
  local d="$WORK/smoke"
  in_lib "$ROOT" "deploy_smoke v0.57.1 '$d'" || fail "smoke of v0.57.1 failed"
  pgrep -f "$d/" >/dev/null && fail "a coordinator still runs for the smoke data dir"
  if in_lib "$ROOT" "deploy_smoke v0.0.0-none.1 '$WORK/smoke-none'" 2>"$WORK/err"; then
    fail "smoke passed for an unpublished tag"
  fi
  grep -q "download" "$WORK/err" || fail "unpublished-tag failure not named: $(cat "$WORK/err")"
}

# run_linux <platform> <binary> — `<binary> --version` inside ubuntu:22.04; the
# binary goes in over stdin because colima shares only $HOME with its VM.
run_linux() {
  docker run -i --rm --platform "$1" ubuntu:22.04 \
    sh -c 'cat >/tmp/comemory && chmod +x /tmp/comemory && /tmp/comemory --version' <"$2"
}

# check_archive <dir> <target> <version> — layout, checksum and `--version` of one archive.
check_archive() {
  local dir="$1" t="$2" v="$3" x="$WORK/x-$2" listing out
  listing="$(tar -tJf "$dir/comemory-$t.tar.xz" | sed 's#/$##' | sort | tr '\n' ' ')"
  [[ "$listing" == "comemory-$t comemory-$t/CHANGELOG.md comemory-$t/LICENSE comemory-$t/README.md comemory-$t/comemory " ]] \
    || fail "$t archive layout: $listing"
  (cd "$dir" && shasum -a 256 -c "comemory-$t.tar.xz.sha256" >/dev/null) || fail "$t .sha256 mismatch"
  mkdir -p "$x"; tar -xJf "$dir/comemory-$t.tar.xz" -C "$x"
  case "$t" in
    aarch64-apple-darwin) out="$("$x/comemory-$t/comemory" --version)" ;;
    aarch64-*) out="$(run_linux linux/arm64 "$x/comemory-$t/comemory")" ;;
    x86_64-*) out="$(run_linux linux/amd64 "$x/comemory-$t/comemory")" ;;
  esac
  [[ "$out" == "comemory $v" ]] || fail "$t binary reports '$out'"
}

case_rehearsal() {
  local c cur tag dir before after out argv t a
  c="$(fresh_clone rehearsal)"; cur="$(version_of "$c")"
  git -C "$c" checkout --quiet main
  tag="v$(bumped "$cur" minor)"; dir="$c/target/deploy/$tag"
  before="$(state "$c"; git -C "$c" ls-remote origin)"
  out="$(cd "$c" && bash "$DEPLOY" --tag "$tag" \
    --change-log $'### Changed\n\n- dry-run rehearsal' --dry-run)" || fail "dry run failed"
  after="$(state "$c"; git -C "$c" ls-remote origin)"
  [[ "$before" == "$after" ]] || fail "dry run changed refs, files or the remote"
  [[ "$(worktrees "$c")" -eq 1 ]] || fail "temp worktree left behind"
  for t in "${TARGETS[@]}"; do check_archive "$dir" "$t" "${tag#v}"; done
  (cd "$dir" && shasum -a 256 -c sha256.sum >/dev/null) || fail "sha256.sum mismatch"
  cmp -s "$dir/install.sh" "$c/install.sh" || fail "install.sh differs from the repo's"
  cmp -s "$dir/install.sh" "$dir/comemory-installer.sh" || fail "comemory-installer.sh differs"
  grep -qF "## [${tag#v}]" "$dir/notes.md" || fail "notes.md lacks the section"
  grep -qx "  version \"${tag#v}\"" "$dir/comemory.rb" || fail "staged formula has the wrong version"
  for t in "${TARGETS[@]}"; do
    grep -qF "sha256 \"$(cut -d' ' -f1 <"$dir/comemory-$t.tar.xz.sha256")\"" "$dir/comemory.rb" \
      || fail "staged formula lacks the $t checksum"
  done
  for a in "run the gates" "git push --atomic" "gh release create" "Homebrew tap"; do
    grep -qF "$a" <<<"$out" || fail "dry run did not name the skipped '$a'"
  done
  argv="$(DEPLOY_TAG="$tag" DEPLOY_ASSETS="$dir" in_lib "$c" deploy_release_argv)"
  for a in --verify-tag --notes-file --latest "$dir/sha256.sum" "$dir/install.sh" \
    "$dir/comemory-installer.sh" "$dir/comemory.rb" "${TARGETS[@]/#/$dir/comemory-}"; do
    grep -qF -- "$a" <<<"$argv" || fail "release argv lacks $a"
  done
  grep -qx -- --prerelease <<<"$argv" && fail "release argv marks a pre-release"
  return 0
}

cases=("$@")
[[ ${#cases[@]} -gt 0 ]] || cases=(args preflight bump formula push skill)
for CASE in "${cases[@]}"; do
  declare -F "case_$CASE" >/dev/null || fail "unknown case"
  "case_$CASE"
  printf 'ok %s\n' "$CASE"
done
