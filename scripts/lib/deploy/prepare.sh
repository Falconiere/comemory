#!/usr/bin/env bash
# scripts/lib/deploy/prepare.sh — argument rules, preflight, local gates and
# the release bump commit for scripts/deploy.sh. Sourced after
# scripts/lib/common.sh; defines functions only.
#
# Inputs (globals): DEPLOY_TAG (optional; asserts the computed tag), DEPLOY_DRY_RUN.
# Outputs (globals): DEPLOY_TAG, DEPLOY_VERSION, DEPLOY_WORKTREE, DEPLOY_COMMIT.

DEPLOY_SCRIPTS="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DEPLOY_REMOTE="${DEPLOY_REMOTE:-origin}"
# The toolchain CI pins (the latest stable when chosen) and every release build uses.
DEPLOY_RUST="1.99.1"
# The version and CHANGELOG come from release-plz, as the release-PR bot made
# them: 0.3.169 is .github/workflows/release-plz.yml's pin (0.4 reworked the
# changelog config). Its semver check needs a cargo-semver-checks that reads
# DEPLOY_RUST's rustdoc JSON: 0.51.0 does for 1.99.1 (the action's 0.50 was
# picked for 1.95).
DEPLOY_RELEASE_PLZ="0.3.169"
DEPLOY_SEMVER_CHECKS="0.51.0"

# deploy_with_rust <cmd...> — run <cmd> on DEPLOY_RUST. RUSTUP_TOOLCHAIN alone
# is not enough: a Homebrew cargo first on PATH ignores it (and so does
# `rustup run`), so the toolchain's own bin goes first.
deploy_with_rust() {
  local cargo
  cargo="$(rustup which --toolchain "$DEPLOY_RUST" cargo 2>/dev/null)" \
    || die deploy:rust "Rust $DEPLOY_RUST is not installed; run: rustup toolchain install $DEPLOY_RUST --component clippy rustfmt"
  PATH="${cargo%/cargo}:$PATH" RUSTUP_TOOLCHAIN="$DEPLOY_RUST" "$@"
}

# deploy_cargo_version <dir> — the [package] version (the first `version =`).
deploy_cargo_version() {
  awk -F'"' '/^version[[:space:]]*=/ { print $2; exit }' "$1/Cargo.toml"
}

# deploy_version_gt <a> <b> — true when X.Y.Z <a> is newer than <b>'s X.Y.Z part.
deploy_version_gt() {
  local a b i
  IFS=. read -r -a a <<<"$1"
  IFS=. read -r -a b <<<"${2%%-*}"
  for i in 0 1 2; do
    if ((10#${a[i]} > 10#${b[i]})); then return 0; fi
    if ((10#${a[i]} < 10#${b[i]})); then return 1; fi
  done
  return 1
}

# --tag is optional: release-plz computes the version, and a given tag only asserts it.
deploy_validate_args() {
  # shortcut: stable tags only; a release candidate would need a -rc.N tag rule here.
  [[ -z "${DEPLOY_TAG:-}" || "$DEPLOY_TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] \
    || die deploy:args "--tag needs a stable vX.Y.Z, got '$DEPLOY_TAG'"
}

# deploy_check_new_tag — refuse a DEPLOY_TAG that exists here or on the remote.
deploy_check_new_tag() {
  local rc=0
  if git -C "$PROJECT_ROOT" rev-parse -q --verify "refs/tags/$DEPLOY_TAG" >/dev/null; then
    die deploy:bump "tag $DEPLOY_TAG already exists locally"
  fi
  git -C "$PROJECT_ROOT" ls-remote --exit-code --tags "$DEPLOY_REMOTE" "refs/tags/$DEPLOY_TAG" \
    >/dev/null 2>&1 || rc=$?
  case "$rc" in
    0) die deploy:bump "tag $DEPLOY_TAG already exists on $DEPLOY_REMOTE" ;;
    2) ;;
    *) die deploy:bump "cannot list tags on $DEPLOY_REMOTE (git ls-remote exit $rc)" ;;
  esac
}

# The bump's two tools, each at its pinned version (never installed from here).
deploy_require_release_tools() {
  [[ "$(release-plz --version 2>/dev/null)" == "release-plz $DEPLOY_RELEASE_PLZ" ]] \
    || die deploy:preflight "needs release-plz $DEPLOY_RELEASE_PLZ; run: cargo install --locked release-plz --version $DEPLOY_RELEASE_PLZ"
  [[ "$(cargo-semver-checks --version 2>/dev/null)" == "cargo-semver-checks $DEPLOY_SEMVER_CHECKS" ]] \
    || die deploy:preflight "needs cargo-semver-checks $DEPLOY_SEMVER_CHECKS; run: cargo install --locked cargo-semver-checks --version $DEPLOY_SEMVER_CHECKS"
}

# deploy_unreleased_empty <changelog> — true when `## [Unreleased]` holds no entry.
deploy_unreleased_empty() {
  awk '/^## \[Unreleased\]/ { inside = 1; next }
       inside && /^## \[/ { exit }
       inside && NF { found = 1 }
       END { exit found }' "$1"
}

deploy_require_gh() {
  require_cmd gh "brew install gh"
  if gh api user --jq .login >/dev/null 2>&1; then return 0; fi
  if [[ -n "${GH_TOKEN:-}" ]]; then
    die deploy:preflight "gh rejects GH_TOKEN; re-run with: env -u GH_TOKEN bash scripts/deploy.sh ..."
  fi
  die deploy:preflight "gh is not logged in; run: gh auth login"
}

deploy_preflight() {
  local root="$PROJECT_ROOT" branch
  [[ -z "$(git -C "$root" status --porcelain --untracked-files=no)" ]] \
    || die deploy:preflight "tracked files are modified; commit or stash them first"
  branch="$(git -C "$root" symbolic-ref --quiet --short HEAD || true)"
  [[ "$branch" == main ]] || die deploy:preflight "releases come from main; current: ${branch:-detached HEAD}"
  grep -q '^## \[Unreleased\]' "$root/CHANGELOG.md" \
    || die deploy:preflight "CHANGELOG.md has no '## [Unreleased]' heading"
  deploy_unreleased_empty "$root/CHANGELOG.md" \
    || die deploy:preflight "CHANGELOG.md [Unreleased] has entries; release-plz writes the section from the commits, so remove them first"
  git -C "$root" fetch --quiet "$DEPLOY_REMOTE" main \
    || die deploy:preflight "cannot fetch $DEPLOY_REMOTE main"
  [[ "$(git -C "$root" rev-parse HEAD)" == "$(git -C "$root" rev-parse FETCH_HEAD)" ]] \
    || die deploy:preflight "main is not equal to $DEPLOY_REMOTE/main; pull or push first"
  [[ "$(uname -sm)" == "Darwin arm64" ]] \
    || die deploy:preflight "builds aarch64-apple-darwin natively, so it must run on an Apple Silicon Mac"
  deploy_with_rust true
  deploy_require_release_tools
  deploy_require_gh
  docker info >/dev/null 2>&1 || die deploy:preflight "docker daemon not reachable; start Docker"
}

# Actions are disabled, so these local gates are all that stands before main.
# They run after the bump, so "nothing to release" refuses in seconds, not after them.
deploy_run_gates() {
  if ((DEPLOY_DRY_RUN)); then
    log_info deploy:gates "skipped in --dry-run; a real run runs them before building"
    return 0
  fi
  log_info deploy:gates "running the full local gate set"
  (cd "$PROJECT_ROOT" \
    && deploy_with_rust bash scripts/check-all.sh \
    && deploy_with_rust cargo nextest run --all-features \
    && deploy_with_rust bash scripts/dup-check.sh \
    && deploy_with_rust bash scripts/deny-check.sh \
    && deploy_with_rust bash scripts/machete-check.sh) \
    || die deploy:gates "a quality gate failed; nothing was released"
  log_ok deploy:gates
}

# deploy_section_has_groups <changelog> <version> — true when `## [<version>]`
# has at least one `### ` group, i.e. some commit rendered a changelog line.
deploy_section_has_groups() {
  awk -v h="## [$2]" 'index($0, h) == 1 { on = 1; next }
       on && /^## \[/ { exit }
       on && /^### / { found = 1; exit }
       END { exit !found }' "$1"
}

# The bump commit is made in a detached temp worktree, so the caller's branch
# only moves when a real (non --dry-run) push succeeds. release-plz computes the
# version (semver-checks decides minor vs patch on 0.x) and writes the section.
deploy_prepare_bump() {
  local wt current changed
  wt="$(mktemp -d "${TMPDIR:-/tmp}/comemory-deploy.XXXXXX")"
  DEPLOY_WORKTREE="$wt/src"
  git -C "$PROJECT_ROOT" worktree add --quiet --detach "$DEPLOY_WORKTREE" HEAD \
    || die deploy:bump "cannot create a temp worktree"
  current="$(deploy_cargo_version "$PROJECT_ROOT")"
  # PR links render from origin's GitHub URL, as in CI (`--repo-url` does not reach them).
  (cd "$DEPLOY_WORKTREE" && deploy_with_rust release-plz update) \
    || die deploy:bump "release-plz update failed (output above)"
  DEPLOY_VERSION="$(deploy_cargo_version "$DEPLOY_WORKTREE")"
  if [[ "$DEPLOY_VERSION" == "$current" ]] \
    || ! deploy_section_has_groups "$DEPLOY_WORKTREE/CHANGELOG.md" "$DEPLOY_VERSION"; then
    die deploy:bump "nothing to release: no commit since v$current renders a changelog line"
  fi
  if [[ -n "${DEPLOY_TAG:-}" && "$DEPLOY_TAG" != "v$DEPLOY_VERSION" ]]; then
    die deploy:bump "release-plz computes v$DEPLOY_VERSION, not the asserted $DEPLOY_TAG"
  fi
  DEPLOY_TAG="v$DEPLOY_VERSION"
  deploy_check_new_tag
  changed="$(git -C "$DEPLOY_WORKTREE" status --porcelain | awk '{ print $2 }' | LC_ALL=C sort | tr '\n' ' ')"
  [[ "$changed" == "CHANGELOG.md Cargo.lock Cargo.toml " ]] \
    || die deploy:bump "release-plz changed more than the bump's three files: $changed"
  git -C "$DEPLOY_WORKTREE" add Cargo.toml Cargo.lock CHANGELOG.md
  git -C "$DEPLOY_WORKTREE" commit --quiet -m "chore(release): $DEPLOY_TAG" \
    -m "Intent: publish the verified stable release from this machine" \
    || die deploy:bump "release commit refused (see the hook output above)"
  DEPLOY_COMMIT="$(git -C "$DEPLOY_WORKTREE" rev-parse HEAD)"
  (cd "$DEPLOY_WORKTREE" && RELEASE_BRANCH=HEAD bash "$DEPLOY_SCRIPTS/validate-release.sh" "$DEPLOY_VERSION") \
    || die deploy:bump "the release commit fails validate-release.sh"
  log_ok deploy:bump "$DEPLOY_TAG at ${DEPLOY_COMMIT:0:12}"
}

# EXIT trap: drop the temp worktree; its commit stays reachable only through a pushed tag.
deploy_cleanup() {
  [[ -n "${DEPLOY_WORKTREE:-}" && -d "$DEPLOY_WORKTREE" ]] || return 0
  git -C "$PROJECT_ROOT" worktree remove --force "$DEPLOY_WORKTREE" \
    || log_err deploy:cleanup "could not remove $DEPLOY_WORKTREE; run: git worktree prune"
  rm -rf "$(dirname "$DEPLOY_WORKTREE")"
  git -C "$PROJECT_ROOT" worktree prune
}
