#!/usr/bin/env bash
# scripts/lib/test_deploy.sh — fixtures and assertions for scripts/test-deploy.sh.
# Sourced by the runner; defines functions only. Kept outside scripts/lib/deploy/
# so deploy.sh and the runner's in_lib glob never load it.
#
# Inputs (globals set by the runner): ROOT, DEPLOY, WORK, CASE.

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

# in_order <text> <needle...> — each needle is on a later line of <text> than the one before.
in_order() {
  local text="$1" prev=0 n needle
  shift
  for needle in "$@"; do
    n="$(grep -nF -- "$needle" <<<"$text" | awk -F: -v p="$prev" '$1 > p { print $1; exit }')" || n=""
    [[ -n "$n" ]] || fail "'$needle' missing after line $prev of: $text"
    prev="$n"
  done
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
  listing="$(tar -tJf "$dir/comemory-$t.tar.xz" | sed 's#/$##' | LC_ALL=C sort | tr '\n' ' ')"
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
  if [[ "$t" == *-linux-gnu ]]; then
    grep -aqE 'GLIBC_2\.(3[5-9]|[4-9][0-9])' "$x/comemory-$t/comemory" \
      && fail "$t binary needs a glibc above the published 2.34 floor"
  fi
  return 0
}

# latest_tag — the tag GitHub's releases/latest redirect points at right now.
latest_tag() {
  local url
  url="$(curl -fsSLI -o /dev/null -w '%{url_effective}' https://github.com/Falconiere/comemory/releases/latest)"
  printf '%s\n' "${url##*/}"
}

# fetch_release <tag> <assets-dir> <json> — the published assets deploy.sh
# uploads (plus source.tar.gz when the release has it, since older sha256.sum
# files list it), notes.md from CHANGELOG.md, and the release's own JSON.
fetch_release() {
  mkdir -p "$2"
  gh release download "$1" --repo Falconiere/comemory --dir "$2" --pattern 'comemory*' \
    --pattern sha256.sum --pattern install.sh --pattern source.tar.gz || fail "cannot download $1"
  bash "$ROOT/scripts/curate-release-notes.sh" "${1#v}" >"$2/notes.md" || fail "no notes for $1"
  gh release view "$1" --repo Falconiere/comemory --json assets,isDraft >"$3" || fail "cannot view $1"
}
