#!/usr/bin/env bash
# scripts/test-deploy.sh — real-data self-check for scripts/deploy.sh.
#
# Every case that touches a repository runs against a fresh `git clone` of this
# checkout whose `origin` is this local repository — except `resume`, which
# fetches from GitHub with no push URL — so no case can push to GitHub.
# Uncommitted work never reaches the clone.
#
# Usage: bash scripts/test-deploy.sh [case...]
#   args preflight bump formula push skill toolchain   fast, offline (the default set)
#   rehearsal   real three-target build through `--dry-run` (Docker, slow)
#   smoke       installs the already-published v0.57.1 release (network)
#   plan        release planner over the latest real release (network, read-only)
#   resume      `--resume` of the latest release; any GitHub write fails it (network, Docker)
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

# shellcheck source=scripts/lib/test_deploy.sh
source "$ROOT/scripts/lib/test_deploy.sh"

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
  # --resume: conflicts are usage errors; every preflight refusal writes nothing.
  local tags newest older
  tags="$(git -C "$c" ls-remote --tags --refs origin | sed 's#.*refs/tags/##' \
    | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' | sort -t. -k1.2,1n -k2,2n -k3,3n)"
  newest="$(tail -1 <<<"$tags")"; older="$(tail -2 <<<"$tags" | head -1)"
  expect_refusal "$c" 64 "--resume needs a value" --resume
  expect_refusal "$c" 64 "--resume excludes" --resume "$newest" --tag "v$next"
  expect_refusal "$c" 64 "--resume excludes" --resume "$newest" --change-log x
  expect_refusal "$c" 64 "--resume excludes" --dry-run --resume "$newest"
  expect_refusal "$c" 1 "--resume needs a stable vX.Y.Z" --resume "$newest-rc.1"
  expect_refusal "$c" 1 "v9.9.9 is not on origin" --resume v9.9.9
  expect_refusal "$c" 1 "$older is not the newest release tag on origin ($newest is)" --resume "$older"
  expect_refusal "$c" 1 "no target/deploy/$newest/" --resume "$newest"
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
  local c bare other cur next later commit out
  c="$(fresh_clone push)"; bare="$WORK/remote.git"
  git -C "$c" checkout --quiet main
  git clone --quiet --bare "$c" "$bare"
  git -C "$c" remote set-url origin "$bare"; git -C "$c" fetch --quiet origin
  cur="$(version_of "$c")"; next="$(bumped "$cur" minor)"; later="$(bumped "$next" patch)"
  # A real pre-push hook records the cargo lefthook's check-all would lint with
  # (on this Mac a Homebrew cargo first on PATH would otherwise win).
  mkdir -p "$WORK/push-hooks"
  printf '#!/bin/sh\ncommand -v cargo >"%s"\n' "$WORK/push-toolchain" >"$WORK/push-hooks/pre-push"
  chmod +x "$WORK/push-hooks/pre-push"
  git -C "$c" config core.hooksPath "$WORK/push-hooks"
  unset RUSTUP_TOOLCHAIN # the deploy must pin it itself, whatever the caller's shell has
  commit="$(DEPLOY_TAG="v$next" DEPLOY_NOTES=x in_lib "$c" \
    'trap deploy_cleanup EXIT; deploy_validate_args; deploy_prepare_bump >&2
     deploy_push >&2; printf "%s\n" "$DEPLOY_COMMIT"')" || fail "push failed"
  local rust; rust="$(in_lib "$c" 'printf %s "$DEPLOY_RUST"')"
  [[ "$(cat "$WORK/push-toolchain")" == "$(rustup which --toolchain "$rust" cargo)" ]] \
    || fail "pre-push hook ran $(cat "$WORK/push-toolchain"), not rustup's $rust cargo"
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
  local rc=0
  DEPLOY_TAG="v$next" in_lib "$c" 'deploy_published_failure "release create failed"' 2>"$WORK/err" || rc=$?
  [[ $rc -eq 2 ]] || fail "a post-push failure exited $rc, expected 2"
  in_order "$(cat "$WORK/err")" "release create failed" "bash scripts/deploy.sh --resume v$next" \
    "gh release delete v$next --repo Falconiere/comemory --yes" \
    "git push --delete origin v$next && git tag --delete v$next"
  out="$(DEPLOY_TAG="v$next" DEPLOY_COMMIT="$commit" DEPLOY_ASSETS="$WORK/assets" \
    in_lib "$c" deploy_print_skipped)" || fail "dry-run list failed"
  in_order "$out" "git push --atomic origin" "--resume v$next" "gh release create v$next" \
    "Homebrew tap" "releases/latest" "smoke: install.sh --version v$next"
}

case_skill() {
  local f="$ROOT/.toolu/skills/deploy/SKILL.md" s
  [[ -f "$f" ]] || fail "missing $f"
  grep -qx 'name: deploy' "$f" || fail "frontmatter name"
  # Claude Code turns .claude/skills/<name> into /<name>; the link keeps one copy.
  [[ "$(readlink "$ROOT/.claude/skills/deploy")" == ../../.toolu/skills/deploy ]] \
    || fail ".claude/skills/deploy must link to ../../.toolu/skills/deploy"
  [[ -f "$ROOT/.claude/skills/deploy/SKILL.md" ]] || fail ".claude/skills/deploy link is broken"
  grep -q '^description: Use when' "$f" || fail "description must start with 'Use when'"
  for s in '## When to Use' '## Procedure' '## Pitfalls' '## Verification'; do
    grep -qx "$s" "$f" || fail "missing section $s"
  done
  for s in 'scripts/deploy.sh --tag' '--dry-run' '--resume vX.Y.Z' 'env -u GH_TOKEN' 'git log'; do
    grep -qF -- "$s" "$f" || fail "skill does not mention '$s'"
  done
}

# The pin must hold whatever PATH puts first: a Homebrew cargo ignores RUSTUP_TOOLCHAIN.
case_toolchain() {
  local out rc=0 rust
  rust="$(in_lib "$ROOT" 'printf %s "$DEPLOY_RUST"')"
  printf 'plain cargo here: %s (%s)\n' "$(command -v cargo)" "$(cargo --version)" >&2
  # The path, not only the version: Homebrew may ship the same version number.
  out="$(in_lib "$ROOT" 'deploy_with_rust sh -c "command -v cargo"')" || fail "pinned cargo did not resolve"
  [[ "$out" == "$(rustup which --toolchain "$rust" cargo)" ]] || fail "pinned cargo is $out, not rustup's $rust"
  out="$(in_lib "$ROOT" 'deploy_with_rust cargo clippy --version')" || fail "pinned clippy did not run"
  [[ "$out" == "clippy 0.1.$(cut -d. -f2 <<<"$rust") "* ]] || fail "pinned clippy is '$out', expected $rust's"
  in_lib "$ROOT" 'DEPLOY_RUST=0.0.1; deploy_with_rust true' 2>"$WORK/err" || rc=$?
  [[ $rc -eq 1 ]] && grep -qF 'rustup toolchain install 0.0.1' "$WORK/err" \
    || fail "a missing toolchain exited $rc: $(cat "$WORK/err")"
}

case_plan() {
  local tag dir rel="$WORK/release.json" out first rc=0
  tag="$(latest_tag)"; dir="$WORK/assets/$tag"
  fetch_release "$tag" "$dir" "$rel"
  # GitHub's own answers: an absent release is 1 (not an error), the real one 0.
  DEPLOY_TAG=v0.0.0 in_lib "$ROOT" "deploy_release_json '$WORK/none.json'" 2>"$WORK/err" || rc=$?
  [[ $rc -eq 1 ]] || fail "an absent release returned $rc, expected 1: $(cat "$WORK/err")"
  DEPLOY_TAG="$tag" in_lib "$ROOT" "deploy_release_json '$WORK/view.json'" || fail "release $tag not found"
  jq -e '.assets | length > 0' "$WORK/view.json" >/dev/null || fail "release $tag JSON lists no assets"
  # The real tap is ahead of 0.0.1, so finishing 0.0.1 stops (exit 2) instead of downgrading it.
  rc=0
  DEPLOY_TAG=v0.0.1 DEPLOY_VERSION=0.0.1 in_lib "$ROOT" deploy_finish_tap 2>"$WORK/err" || rc=$?
  [[ $rc -eq 2 ]] && grep -q "ahead of 0.0.1" "$WORK/err" || fail "tap downgrade not refused ($rc): $(cat "$WORK/err")"
  plan_for() { DEPLOY_TAG="$tag" DEPLOY_ASSETS="$dir" in_lib "$ROOT" "deploy_release_plan '$1'"; }
  out="$(plan_for "$rel")" || fail "planning the complete $tag release failed"
  [[ -z "$out" ]] || fail "the complete $tag release still planned: $out"
  [[ "$(plan_for "")" == create ]] || fail "an absent release is not planned as create"
  jq '.assets |= map(select(.name != "install.sh"))' "$rel" >"$WORK/missing.json"
  [[ "$(plan_for "$WORK/missing.json")" == "upload $dir/install.sh" ]] || fail "missing asset not uploaded"
  jq '(.assets[] | select(.name == "sha256.sum")).state = "new"' "$rel" >"$WORK/partial.json"
  [[ "$(plan_for "$WORK/partial.json")" == "upload $dir/sha256.sum" ]] || fail "half-uploaded asset not re-uploaded"
  jq '.isDraft = true' "$rel" >"$WORK/draft.json"
  plan_for "$WORK/draft.json" >/dev/null 2>&1 && fail "a draft release was planned"
  first="comemory-${TARGETS[0]}.tar.xz"
  printf x >>"$dir/$first"
  if out="$(plan_for "$rel" 2>"$WORK/err")"; then fail "a tampered $first was planned: $out"; fi
  [[ -z "$out" ]] || fail "actions printed before the digest refusal: $out"
  grep -q digest "$WORK/err" && grep -qF "$first" "$WORK/err" || fail "refusal not named: $(cat "$WORK/err")"
}

# The clone fetches from GitHub but has no push URL, and a gh guard first on
# PATH records and refuses every write, so a misjudged step fails this case
# instead of publishing anything.
case_resume() {
  local c tag newest out real_gh guard="$WORK/guard"
  tag="$(latest_tag)"; c="$(fresh_clone resume)"
  git -C "$c" remote set-url origin https://github.com/Falconiere/comemory.git
  git -C "$c" remote set-url --push origin "$WORK/no-push.git"
  newest="$(in_lib "$c" 'deploy_newest_tag "$(git ls-remote --tags --refs origin)"')"
  [[ "$newest" == "$tag" ]] \
    || fail "releases/latest is $tag but the newest tag is $newest: a release is half-published"
  fetch_release "$tag" "$c/target/deploy/$tag" "$WORK/release.json"
  real_gh="$(command -v gh)"; mkdir -p "$guard"
  printf '#!/bin/sh\ncase "$1 $2" in\n  "release create"|"release upload"|"release edit"|"repo clone")\n    printf "%%s\\n" "$*" >>"%s"; exit 1 ;;\nesac\nexec "%s" "$@"\n' \
    "$WORK/gh-writes" "$real_gh" >"$guard/gh"
  chmod +x "$guard/gh"
  out="$(cd "$c" && PATH="$guard:$PATH" bash "$DEPLOY" --resume "$tag" 2>&1)" || fail "resume of $tag failed: $out"
  [[ ! -e "$WORK/gh-writes" ]] || fail "resume attempted GitHub writes: $(cat "$WORK/gh-writes")"
  in_order "$out" "[deploy:resume] OK $tag" "[deploy:release] OK already complete" \
    "[deploy:tap] OK already at ${tag#v}" "[deploy:latest] OK $tag" "[deploy:smoke] OK $tag" \
    "https://github.com/Falconiere/comemory/releases/tag/$tag"
}

case_smoke() {
  local d="$WORK/smoke" latest
  latest="$(curl -fsSLI -o /dev/null -w '%{url_effective}' https://github.com/Falconiere/comemory/releases/latest)"
  in_lib "$ROOT" "deploy_latest_is '${latest##*/}'" || fail "deploy_latest_is rejects the real latest ${latest##*/}"
  in_lib "$ROOT" "deploy_latest_is v0.0.0" && fail "deploy_latest_is accepted v0.0.0"
  in_lib "$ROOT" "deploy_smoke v0.57.1 '$d'" || fail "smoke of v0.57.1 failed"
  pgrep -f "$d/" >/dev/null && fail "a coordinator still runs for the smoke data dir"
  if in_lib "$ROOT" "deploy_smoke v0.0.0-none.1 '$WORK/smoke-none'" 2>"$WORK/err"; then
    fail "smoke passed for an unpublished tag"
  fi
  grep -q "download" "$WORK/err" || fail "unpublished-tag failure not named: $(cat "$WORK/err")"
}

case_rehearsal() {
  local c cur tag dir before after out argv t a
  c="$(fresh_clone rehearsal)"; cur="$(version_of "$c")"
  git -C "$c" checkout --quiet main
  # A bare snapshot as origin: this checkout's own refs may move during the long build.
  git clone --quiet --bare "$c" "$WORK/rehearsal-origin.git"
  git -C "$c" remote set-url origin "$WORK/rehearsal-origin.git"
  tag="v$(bumped "$cur" minor)"; dir="$c/target/deploy/$tag"
  before="$(state "$c"; git -C "$c" ls-remote origin)"
  out="$(cd "$c" && bash "$DEPLOY" --tag "$tag" \
    --change-log $'### Changed\n\n- dry-run rehearsal' --dry-run)" || fail "dry run failed"
  after="$(state "$c"; git -C "$c" ls-remote origin)"
  [[ "$before" == "$after" ]] \
    || fail "dry run changed refs, files or the remote: $(diff <(echo "$before") <(echo "$after"))"
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
  # A measurement, not a bound: a cold cache or another session's Docker load must not fail it.
  t="$(grep -oE '\[deploy:build\] OK linux \([0-9]+s\)' <<<"$out")" || fail "no linux build time logged"
  printf 'rehearsal %s\n' "$t" >&2
  argv="$(DEPLOY_TAG="$tag" DEPLOY_ASSETS="$dir" in_lib "$c" deploy_release_argv)"
  grep -A1 -x -- --repo <<<"$argv" | grep -qx Falconiere/comemory || fail "release argv lacks --repo"
  for a in --verify-tag --notes-file --latest "$dir/sha256.sum" "$dir/install.sh" \
    "$dir/comemory-installer.sh" "$dir/comemory.rb" "${TARGETS[@]/#/$dir/comemory-}"; do
    grep -qF -- "$a" <<<"$argv" || fail "release argv lacks $a"
  done
  grep -qx -- --prerelease <<<"$argv" && fail "release argv marks a pre-release"
  return 0
}

cases=("$@")
[[ ${#cases[@]} -gt 0 ]] || cases=(args preflight bump formula push skill toolchain)
for CASE in "${cases[@]}"; do
  declare -F "case_$CASE" >/dev/null || fail "unknown case"
  "case_$CASE"
  printf 'ok %s\n' "$CASE"
done
