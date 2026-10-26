#!/usr/bin/env bash
# scripts/lib/deploy/build.sh — build, package and verify the release archives
# for scripts/deploy.sh. Sourced after prepare.sh; defines functions only.
#
# macOS builds in the bump commit's temp worktree (DEPLOY_WORKTREE), Linux from
# `git archive` of that commit (DEPLOY_COMMIT); every asset is written to
# DEPLOY_ASSETS = <repo>/target/deploy/<tag>/.

DEPLOY_TARGETS=(aarch64-apple-darwin aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu)
# Linux builds run on ubuntu:22.04 (glibc 2.35), the image cargo-dist's runners
# used; the binaries reference GLIBC_2.34 at most, the published floor. Older
# Debian images cannot link close_range (glibc 2.34+, used by the sync daemon's
# spawn), and bookworm could raise the floor to 2.36.
DEPLOY_RUN_IMAGE="ubuntu:22.04"

# deploy_platform <target> — the Docker platform a Linux target builds and runs on.
deploy_platform() {
  case "$1" in
    aarch64-unknown-linux-gnu) printf 'linux/arm64\n' ;;
    x86_64-unknown-linux-gnu) printf 'linux/amd64\n' ;;
    *) return 1 ;;
  esac
}

deploy_build_all() {
  local t
  DEPLOY_ASSETS="$PROJECT_ROOT/target/deploy/$DEPLOY_TAG"
  rm -rf "${DEPLOY_ASSETS:?}"
  mkdir -p "$DEPLOY_ASSETS/bin"
  for t in "${DEPLOY_TARGETS[@]}"; do
    log_info deploy:build "$t"
    if [[ "$t" == aarch64-apple-darwin ]]; then deploy_build_macos "$t"; else deploy_build_linux "$t"; fi
    log_ok deploy:build "$t"
  done
}

deploy_build_macos() {
  local cache="$PROJECT_ROOT/target/deploy-cache/macos"
  (cd "$DEPLOY_WORKTREE" && RUSTUP_TOOLCHAIN="$DEPLOY_RUST" CARGO_TARGET_DIR="$cache" \
    cargo build --release --locked --bin comemory --target "$1") \
    || die deploy:build "$1 build failed"
  cp "$cache/$1/release/comemory" "$DEPLOY_ASSETS/bin/comemory-$1"
}

# No bind mounts: colima and similar VMs share only $HOME, so a mounted repo
# under /Volumes or $TMPDIR arrives empty. The release commit's tree goes in
# over stdin and the binary comes back on stdout. Named volumes keep the
# target dir and registry warm; the source always sits at /src, so cached
# dependencies stay valid across runs. `tar -m` stamps the sources with the
# VM's own clock, so they are always newer than the cached outputs and cargo
# can never ship a stale binary when host and VM clocks disagree.
deploy_build_linux() {
  local platform arch image
  platform="$(deploy_platform "$1")"
  arch="${platform#linux/}"
  image="comemory-deploy-builder:$DEPLOY_RUST-$arch"
  # Dockerfile on stdin, no build context; layers are cached after the first run.
  if ! docker build --quiet --platform "$platform" --tag "$image" - >/dev/null <<EOF
FROM $DEPLOY_RUN_IMAGE
RUN apt-get update && apt-get install -y --no-install-recommends \\
      build-essential ca-certificates curl pkg-config zlib1g-dev \\
  && rm -rf /var/lib/apt/lists/*
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain $DEPLOY_RUST
ENV PATH=/root/.cargo/bin:\$PATH
EOF
  then
    die deploy:build "cannot build the $image builder image"
  fi
  git -C "$PROJECT_ROOT" archive --format=tar "$DEPLOY_COMMIT" \
    | docker run -i --rm --platform "$platform" \
      -v "comemory-deploy-target-$DEPLOY_RUST-$arch:/src/target" \
      -v "comemory-deploy-registry-$DEPLOY_RUST-$arch:/root/.cargo/registry" \
      "$image" sh -c 'tar -xmf - -C /src && cd /src \
        && cargo build --release --locked --bin comemory >&2 && cat target/release/comemory' \
      >"$DEPLOY_ASSETS/bin/comemory-$1" \
    || die deploy:build "$1 build failed"
}

# Same asset names and archive layout as cargo-dist, so install.sh,
# `comemory upgrade` and the Homebrew formula keep working.
deploy_package() {
  local t stage="$DEPLOY_ASSETS/stage"
  for t in "${DEPLOY_TARGETS[@]}"; do
    rm -rf "$stage"
    mkdir -p "$stage/comemory-$t"
    install -m 0755 "$DEPLOY_ASSETS/bin/comemory-$t" "$stage/comemory-$t/comemory"
    cp "$DEPLOY_WORKTREE/README.md" "$DEPLOY_WORKTREE/CHANGELOG.md" "$DEPLOY_WORKTREE/LICENSE" \
      "$stage/comemory-$t/"
    COPYFILE_DISABLE=1 tar -C "$stage" -cJf "$DEPLOY_ASSETS/comemory-$t.tar.xz" "comemory-$t"
    (cd "$DEPLOY_ASSETS" && shasum -a 256 -b "comemory-$t.tar.xz" >"comemory-$t.tar.xz.sha256")
  done
  rm -rf "$stage" "${DEPLOY_ASSETS:?}/bin"
  (cd "$DEPLOY_ASSETS" && cat comemory-*.tar.xz.sha256 >sha256.sum)
  cp "$DEPLOY_WORKTREE/install.sh" "$DEPLOY_ASSETS/install.sh"
  # get.comemory.io still redirects to cargo-dist's installer name.
  cp "$DEPLOY_WORKTREE/install.sh" "$DEPLOY_ASSETS/comemory-installer.sh"
  (cd "$DEPLOY_WORKTREE" && bash "$DEPLOY_SCRIPTS/curate-release-notes.sh" "$DEPLOY_VERSION") \
    >"$DEPLOY_ASSETS/notes.md" || die deploy:package "cannot extract the release notes"
}

# deploy_binary_version <target> <binary> — `<binary> --version`, run where the target runs.
deploy_binary_version() {
  if [[ "$1" == aarch64-apple-darwin ]]; then
    "$2" --version
  else
    docker run -i --rm --platform "$(deploy_platform "$1")" "$DEPLOY_RUN_IMAGE" \
      sh -c 'cat >/tmp/comemory && chmod +x /tmp/comemory && /tmp/comemory --version' <"$2"
  fi
}

deploy_verify_assets() {
  local t out x="$DEPLOY_ASSETS/verify"
  (cd "$DEPLOY_ASSETS" && shasum -a 256 -c sha256.sum >/dev/null) \
    || die deploy:verify "sha256.sum does not match the archives"
  grep -qF "## [$DEPLOY_VERSION]" "$DEPLOY_ASSETS/notes.md" \
    || die deploy:verify "notes.md lacks the $DEPLOY_VERSION section"
  # Extracted under the asset dir, so a failed check leaves its evidence there.
  rm -rf "$x"
  mkdir -p "$x"
  for t in "${DEPLOY_TARGETS[@]}"; do
    tar -xJf "$DEPLOY_ASSETS/comemory-$t.tar.xz" -C "$x" || die deploy:verify "$t archive does not extract"
    # stdout only: a first `docker run` also prints image-pull progress on stderr.
    out="$(deploy_binary_version "$t" "$x/comemory-$t/comemory" 2>"$x/err")" \
      || die deploy:verify "$t binary does not run: $(cat "$x/err")"
    [[ "$out" == "comemory $DEPLOY_VERSION" ]] \
      || die deploy:verify "$t binary reports '$out', expected 'comemory $DEPLOY_VERSION'"
    log_ok deploy:verify "$t → $out"
  done
  rm -rf "$x"
}
