#!/usr/bin/env bash
# scripts/lib/deploy/publish.sh — push, GitHub Release, Homebrew tap and the
# post-publish smoke install for scripts/deploy.sh. Sourced after build.sh;
# defines functions only.

DEPLOY_REPO="Falconiere/comemory"
DEPLOY_TAP="Falconiere/homebrew-tap"

# deploy_rewrite_formula <formula> <version> <sha256.sum> — print the formula
# moved to <version>: the version line, each archive URL and its sha256.
deploy_rewrite_formula() {
  awk -v v="$2" -v sums="$3" '
    BEGIN {
      while ((getline line < sums) > 0) { split(line, f, " "); name = f[2]; sub(/^\*/, "", name); sha[name] = f[1] }
    }
    /^  version "/ { print "  version \"" v "\""; next }
    /url "https:\/\/github\.com\/.*\/releases\/download\// {
      sub(/\/download\/v[^\/]+\//, "/download/v" v "/")
      match($0, /comemory-[^"]+\.tar\.xz/); asset = substr($0, RSTART, RLENGTH)
      if (!(asset in sha)) { print "no checksum for " asset > "/dev/stderr"; failed = 1; exit 1 }
      print; next
    }
    /sha256 "/ && asset != "" { sub(/sha256 "[0-9a-f]+"/, "sha256 \"" sha[asset] "\""); asset = ""; print; next }
    { print }
    END { if (failed) exit 1 }
  ' "$1"
}

# deploy_tap_formula — print the tap's current Formula/comemory.rb (read-only).
deploy_tap_formula() {
  gh api "repos/$DEPLOY_TAP/contents/Formula/comemory.rb" --jq .content | base64 -d
}

# Write <assets>/comemory.rb from the tap's current formula (read-only).
deploy_stage_formula() {
  local tmp
  tmp="$(mktemp -d)"
  deploy_tap_formula >"$tmp/current.rb" || die deploy:formula "cannot read Formula/comemory.rb from $DEPLOY_TAP"
  deploy_rewrite_formula "$tmp/current.rb" "$DEPLOY_VERSION" "$DEPLOY_ASSETS/sha256.sum" \
    >"$DEPLOY_ASSETS/comemory.rb" || die deploy:formula "cannot rewrite the formula"
  rm -rf "$tmp"
}

# deploy_release_argv — the `gh` arguments of the release, one per line.
deploy_release_argv() {
  printf '%s\n' release create "$DEPLOY_TAG" --repo "$DEPLOY_REPO" --verify-tag \
    --title "$DEPLOY_TAG" --notes-file "$DEPLOY_ASSETS/notes.md" --latest
  deploy_release_assets
}

# deploy_release_assets — the files the release carries, one path per line.
deploy_release_assets() {
  local t
  for t in "${DEPLOY_TARGETS[@]}"; do
    printf '%s\n' "$DEPLOY_ASSETS/comemory-$t.tar.xz" "$DEPLOY_ASSETS/comemory-$t.tar.xz.sha256"
  done
  printf '%s\n' "$DEPLOY_ASSETS/sha256.sum" "$DEPLOY_ASSETS/install.sh" \
    "$DEPLOY_ASSETS/comemory-installer.sh" "$DEPLOY_ASSETS/comemory.rb"
}

# deploy_push_refs — the refspecs one atomic push sends.
deploy_push_refs() {
  printf '%s\n' "$DEPLOY_COMMIT:refs/heads/main" "refs/tags/$DEPLOY_TAG"
}

deploy_print_skipped() {
  log_info deploy:dry-run "skipped — a real run would also:"
  printf '  run the gates: check-all (Rust 1.95), nextest, dup-check, deny-check, machete-check\n'
  printf '  git tag -a %s -m %s %s\n' "$DEPLOY_TAG" "$DEPLOY_TAG" "$DEPLOY_COMMIT"
  printf '  git push --atomic %s %s\n' "$DEPLOY_REMOTE" "$(deploy_push_refs | tr '\n' ' ')"
  printf '  then finish, skipping what is already done (what --resume %s re-runs):\n' "$DEPLOY_TAG"
  printf '    gh %s (or upload only the missing assets)\n' "$(deploy_release_argv | tr '\n' ' ')"
  printf '    Homebrew tap: push %s/comemory.rb to %s unless it is at %s\n' \
    "$DEPLOY_ASSETS" "$DEPLOY_TAP" "${DEPLOY_TAG#v}"
  printf '    mark %s latest unless releases/latest already points at it\n' "$DEPLOY_TAG"
  printf '    smoke: install.sh --version %s into a temp dir\n' "$DEPLOY_TAG"
}

# Tag the bump commit and push it in one atomic push; a rejection leaves nothing behind.
deploy_push() {
  local refs=() ref
  while IFS= read -r ref; do refs+=("$ref"); done < <(deploy_push_refs)
  git -C "$PROJECT_ROOT" tag -a "$DEPLOY_TAG" -m "$DEPLOY_TAG" "$DEPLOY_COMMIT" \
    || die deploy:push "cannot create tag $DEPLOY_TAG"
  # lefthook's pre-push check-all must lint on the CI-pinned toolchain; a newer
  # default clippy rejects main and would refuse every release here.
  if ! RUSTUP_TOOLCHAIN="$DEPLOY_RUST" git -C "$PROJECT_ROOT" push --atomic "$DEPLOY_REMOTE" "${refs[@]}"; then
    git -C "$PROJECT_ROOT" tag -d "$DEPLOY_TAG" >/dev/null
    die deploy:push "push rejected; nothing was published (local tag $DEPLOY_TAG removed)"
  fi
  git -C "$PROJECT_ROOT" merge --ff-only --quiet "$DEPLOY_COMMIT" \
    || log_err deploy:push "pushed, but local main did not fast-forward; run: git pull --ff-only"
  log_ok deploy:push "$(deploy_push_refs | tr '\n' ' ')"
}

# deploy_published_failure <message> — after the push: print how to finish, exit 2.
deploy_published_failure() {
  log_err deploy:publish "$1"
  cat >&2 <<EOF
$DEPLOY_TAG is pushed. Finish it; every step skips what is already done:
  bash scripts/deploy.sh --resume $DEPLOY_TAG
Only to withdraw a bad release (docs/release.md §5):
  gh release delete $DEPLOY_TAG --repo $DEPLOY_REPO --yes
  git push --delete $DEPLOY_REMOTE $DEPLOY_TAG && git tag --delete $DEPLOY_TAG
EOF
  printf '  main already carries chore(release): %s; revert it with a new commit if needed\n' \
    "$DEPLOY_TAG" >&2
  exit 2
}

deploy_publish_tap() {
  local tmp rc=0
  tmp="$(mktemp -d)"
  gh repo clone "$DEPLOY_TAP" "$tmp/tap" -- --quiet --depth 1 \
    && cp "$DEPLOY_ASSETS/comemory.rb" "$tmp/tap/Formula/comemory.rb" \
    && git -C "$tmp/tap" commit --quiet -am "comemory $DEPLOY_VERSION" \
    && git -C "$tmp/tap" -c credential.helper='!gh auth git-credential' push --quiet origin HEAD \
    || rc=1
  rm -rf "$tmp"
  return "$rc"
}

# deploy_smoke <tag> [dir] — install <tag> with the release's own install.sh
# into a temp dir and data dir, check `--version`, then remove its coordinator.
deploy_smoke() {
  local tag="$1" dir="${2:-}" keep=1 out rc=0
  if [[ -z "$dir" ]]; then dir="$(mktemp -d)"; keep=0; fi
  mkdir -p "$dir"
  if ! curl -fsSL "https://github.com/$DEPLOY_REPO/releases/download/$tag/install.sh" -o "$dir/install.sh"; then
    log_err deploy:smoke "cannot download install.sh from release $tag"
    return 1
  fi
  COMEMORY_DATA_DIR="$dir/data" sh "$dir/install.sh" --version "$tag" --dir "$dir/bin" \
    --no-modify-path --no-completions --quiet || rc=1
  if [[ -x "$dir/bin/comemory" ]]; then
    out="$("$dir/bin/comemory" --version)" || rc=1
    [[ "${out:-}" == "comemory ${tag#v}" ]] || { log_err deploy:smoke "installed binary reports '${out:-}'"; rc=1; }
    # launchctl prints "Boot-out failed" for the supervisor-run temp coordinator; only the exit counts.
    out="$("$dir/bin/comemory" --data-dir "$dir/data" sync daemon uninstall 2>&1)" \
      || log_err deploy:smoke "could not uninstall the smoke coordinator for $dir/data: $out"
  fi
  if ((keep == 0)); then rm -rf "$dir"; fi
  if ((rc == 0)); then log_ok deploy:smoke "$tag installs and runs"; fi
  return "$rc"
}

deploy_latest_is() {
  local final
  final="$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$DEPLOY_REPO/releases/latest")"
  [[ "${final##*/}" == "$1" ]]
}

# GitHub's releases/latest redirect can trail a fresh release by a few seconds.
deploy_wait_latest() {
  local i
  for i in 1 2 3 4 5 6; do
    if deploy_latest_is "$1"; then return 0; fi
    sleep "$((i * 5))"
  done
  return 1
}

# The same finish --resume runs, so every real release exercises it.
deploy_publish() {
  deploy_push
  deploy_finish
}
