#!/usr/bin/env bash
# scripts/lib/deploy/resume.sh — the idempotent half of a release, shared by a
# normal run (after its push) and `--resume vX.Y.Z` (after a failed one).
# Sourced after publish.sh; defines functions only.

# deploy_release_plan <release.json|""> — print one action per line for what the
# GitHub Release still lacks: `create`, `upload <path>` (asset absent) or
# `replace <path>` (asset left half-uploaded). "" means there is no release.
# Refuses a draft, and an uploaded asset that is not this exact local file, so
# a release never mixes two builds. Nothing is printed before a refusal.
deploy_release_plan() {
  local json="$1" path prev="" name state digest want actions=()
  if [[ -z "$json" ]]; then printf 'create\n'; return 0; fi
  if jq -e '.isDraft' "$json" >/dev/null; then
    die deploy:resume "$DEPLOY_TAG is a draft release; publish or delete it by hand"
  fi
  while IFS= read -r path; do
    # The asset files of deploy_release_argv; its --notes-file value is not one.
    [[ "$path" == "$DEPLOY_ASSETS"/* && "$prev" != --notes-file ]] || { prev="$path"; continue; }
    prev="$path"
    name="${path##*/}" state="" digest=""
    IFS=$'\t' read -r state digest < <(jq -r --arg n "$name" \
      '.assets[] | select(.name == $n) | [.state, (.digest // "none")] | @tsv' "$json") || true
    if [[ -z "$state" ]]; then actions+=("upload $path"); continue; fi
    if [[ "$state" != uploaded ]]; then actions+=("replace $path"); continue; fi
    want="sha256:$(shasum -a 256 "$path" | cut -d' ' -f1)"
    [[ "$digest" == "$want" ]] \
      || die deploy:resume "$name on the release has digest $digest, the local file $want; refusing to mix builds"
  done < <(deploy_release_argv)
  if ((${#actions[@]})); then printf '%s\n' "${actions[@]}"; fi
}

# deploy_newest_tag <ls-remote output> — the highest stable vX.Y.Z tag in it.
deploy_newest_tag() {
  local ref t newest=""
  while read -r _ ref; do
    t="${ref#refs/tags/}"
    [[ "$t" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || continue
    if [[ -z "$newest" ]] || deploy_version_gt "${t#v}" "${newest#v}"; then newest="$t"; fi
  done <<<"$1"
  printf '%s\n' "$newest"
}

# Resume's preflight: writes nothing, and refuses unless DEPLOY_TAG is the
# newest release tag on the remote, on its main, with this run's assets intact.
deploy_resume_preflight() {
  local root="$PROJECT_ROOT" tags newest commit
  [[ "$DEPLOY_TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] \
    || die deploy:resume "--resume needs a stable vX.Y.Z, got '$DEPLOY_TAG'"
  DEPLOY_VERSION="${DEPLOY_TAG#v}"
  tags="$(git -C "$root" ls-remote --tags --refs "$DEPLOY_REMOTE")" \
    || die deploy:resume "cannot list tags on $DEPLOY_REMOTE"
  grep -q "refs/tags/$DEPLOY_TAG\$" <<<"$tags" || die deploy:resume "$DEPLOY_TAG is not on $DEPLOY_REMOTE; nothing to resume"
  newest="$(deploy_newest_tag "$tags")"
  [[ "$DEPLOY_TAG" == "$newest" ]] \
    || die deploy:resume "$DEPLOY_TAG is not the newest release tag on $DEPLOY_REMOTE ($newest is)"
  # The peeled line follows the tag's own line, so the last one is the commit.
  commit="$(git -C "$root" ls-remote "$DEPLOY_REMOTE" "refs/tags/$DEPLOY_TAG" "refs/tags/$DEPLOY_TAG^{}" | tail -1 | cut -f1)"
  git -C "$root" fetch --quiet --no-tags "$DEPLOY_REMOTE" main || die deploy:resume "cannot fetch $DEPLOY_REMOTE main"
  git -C "$root" merge-base --is-ancestor "$commit" FETCH_HEAD 2>/dev/null \
    || die deploy:resume "$DEPLOY_TAG's commit is not on $DEPLOY_REMOTE/main"
  DEPLOY_ASSETS="$root/target/deploy/$DEPLOY_TAG"
  [[ -d "$DEPLOY_ASSETS" ]] \
    || die deploy:resume "no target/deploy/$DEPLOY_TAG/ from the failed run; resume reuses its assets and never rebuilds"
  deploy_require_gh
  docker info >/dev/null 2>&1 || die deploy:resume "docker daemon not reachable; the Linux binaries are checked in it"
  deploy_verify_assets
  log_ok deploy:resume "$DEPLOY_TAG at ${commit:0:12}; assets verified"
}

# Create the GitHub Release, or upload what an earlier attempt left out.
deploy_finish_release() {
  local tmp json actions verb path argv=() a uploads=() replaces=()
  tmp="$(mktemp -d)"; json="$tmp/release.json"
  if ! gh release view "$DEPLOY_TAG" --repo "$DEPLOY_REPO" --json assets,isDraft >"$json" 2>"$tmp/err"; then
    grep -q 'release not found' "$tmp/err" \
      || deploy_published_failure "cannot read release $DEPLOY_TAG: $(cat "$tmp/err")"
    json=""
  fi
  actions="$(deploy_release_plan "$json")" || deploy_published_failure "release $DEPLOY_TAG cannot be completed"
  rm -rf "$tmp"
  if [[ -z "$actions" ]]; then log_ok deploy:release "already complete"; return 0; fi
  if [[ "$actions" == create ]]; then
    while IFS= read -r a; do argv+=("$a"); done < <(deploy_release_argv)
    gh "${argv[@]}" >/dev/null || deploy_published_failure "gh release create failed"
    log_ok deploy:release "created"; return 0
  fi
  while read -r verb path; do
    if [[ "$verb" == upload ]]; then uploads+=("$path"); else replaces+=("$path"); fi
  done <<<"$actions"
  if ((${#uploads[@]})); then
    gh release upload "$DEPLOY_TAG" --repo "$DEPLOY_REPO" "${uploads[@]}" \
      || deploy_published_failure "uploading ${#uploads[@]} missing asset(s) failed"
  fi
  if ((${#replaces[@]})); then
    gh release upload --clobber "$DEPLOY_TAG" --repo "$DEPLOY_REPO" "${replaces[@]}" \
      || deploy_published_failure "replacing ${#replaces[@]} half-uploaded asset(s) failed"
  fi
  log_ok deploy:release "uploaded $((${#uploads[@]} + ${#replaces[@]}))"
}

# Move the tap to this version unless it is there already; never downgrade it.
deploy_finish_tap() {
  local have
  have="$(deploy_tap_formula | awk -F'"' '/^  version "/ { print $2; exit }')" || have=""
  [[ -n "$have" ]] || deploy_published_failure "cannot read the version of $DEPLOY_TAP's formula"
  if [[ "$have" == "$DEPLOY_VERSION" ]]; then log_ok deploy:tap "already at $have"; return 0; fi
  if deploy_version_gt "$have" "$DEPLOY_VERSION"; then
    deploy_published_failure "$DEPLOY_TAP is at $have, ahead of $DEPLOY_VERSION; not downgrading it"
  fi
  deploy_publish_tap || deploy_published_failure "Homebrew tap not updated"
  log_ok deploy:tap "$DEPLOY_TAP → $DEPLOY_VERSION"
}

deploy_finish_latest() {
  if ! deploy_latest_is "$DEPLOY_TAG"; then
    gh release edit "$DEPLOY_TAG" --repo "$DEPLOY_REPO" --latest >/dev/null \
      || deploy_published_failure "cannot mark $DEPLOY_TAG latest"
    deploy_wait_latest "$DEPLOY_TAG" \
      || deploy_published_failure "releases/latest still does not point at $DEPLOY_TAG after ~2 minutes"
  fi
  log_ok deploy:latest "$DEPLOY_TAG"
}

# Every step after the push, each skipping what is already done; prints the release URL.
deploy_finish() {
  deploy_finish_release
  deploy_finish_tap
  deploy_finish_latest
  deploy_smoke "$DEPLOY_TAG" || deploy_published_failure "smoke install of $DEPLOY_TAG failed"
  printf 'https://github.com/%s/releases/tag/%s\n' "$DEPLOY_REPO" "$DEPLOY_TAG"
}
