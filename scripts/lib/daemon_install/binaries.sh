#!/usr/bin/env bash
# Real binaries for scripts/test-daemon-install.sh (D11): the real published
# v0.50.0 (carries #257's `ensure`) and v0.49.1 (does not) archives, and two
# real branch builds (`X.Y.(Z+1)-lifecycle.1`/`.2`) from a scratch git
# worktree. Everything is cached under target/daemon-install-cache/, keyed so
# a fresh build or a new HEAD invalidates it. Sourced, never executed.

set -euo pipefail

REPO_URL="https://github.com/Falconiere/comemory"
OLD_TAG="v0.50.0"
BROKEN_TAG="v0.49.1"

cache_root() { printf '%s' "${DAEMON_INSTALL_CACHE:-$PROJECT_ROOT/target/daemon-install-cache}"; }

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

# fetch <url> <dest>
fetch() { curl -fsSL -o "$2" "$1"; }

# fetch_real_release <tag> <target> — downloads and verifies the real
# GitHub asset once; returns the cache dir (tarball + sha256 + that
# release's own install.sh) via stdout.
fetch_real_release() {
  local tag=$1 target=$2 dir archive
  dir="$(cache_root)/fetched/$tag/$target"
  archive="comemory-$target.tar.xz"
  if [ -f "$dir/$archive" ] && [ -f "$dir/$archive.sha256" ] && [ -f "$dir/install.sh" ]; then
    printf '%s' "$dir"
    return 0
  fi
  mkdir -p "$dir"
  local base="$REPO_URL/releases/download/$tag"
  fetch "$base/$archive" "$dir/$archive.part"
  fetch "$base/$archive.sha256" "$dir/$archive.sha256.part"
  fetch "$base/install.sh" "$dir/install.sh.part"
  local expected got
  expected="$(cut -d' ' -f1 <"$dir/$archive.sha256.part")"
  got="$(sha256_file "$dir/$archive.part")"
  [ "$expected" = "$got" ] || {
    rm -f "$dir/$archive.part" "$dir/$archive.sha256.part" "$dir/install.sh.part"
    printf 'checksum mismatch for %s: expected %s got %s\n' "$tag/$archive" "$expected" "$got" >&2
    return 1
  }
  mv -f "$dir/$archive.part" "$dir/$archive"
  mv -f "$dir/$archive.sha256.part" "$dir/$archive.sha256"
  mv -f "$dir/install.sh.part" "$dir/install.sh"
  printf '%s' "$dir"
}

# tar_xz_package <bin-path> <target> <dest-dir> — lay `bin-path` out as
# `comemory-<target>/comemory` in `dest-dir/comemory-<target>.tar.xz`, plus
# its sha256 sidecar in cargo-dist's `<hash> *<file>` form.
tar_xz_package() {
  local bin=$1 target=$2 dest=$3
  local pkg_name="comemory-$target" archive stage
  archive="$pkg_name.tar.xz"
  stage="$(mktemp -d)"
  mkdir -p "$stage/$pkg_name"
  cp "$bin" "$stage/$pkg_name/comemory"
  chmod 755 "$stage/$pkg_name/comemory"
  mkdir -p "$dest"
  (cd "$stage" && XZ_OPT='-0 -T0' tar -cJf "$dest/$archive" "$pkg_name")
  printf '%s *%s\n' "$(sha256_file "$dest/$archive")" "$archive" >"$dest/$archive.sha256"
  rm -rf "$stage"
}

# stage_tag <served-root> <tag> <archive-path> <sha256-path> — one release
# under `served-root/<tag>/`, with THIS branch's own install.sh (D11: "the
# served install.sh is always this branch's").
stage_tag() {
  local root=$1 tag=$2 archive=$3 sha=$4 dir
  dir="$root/$tag"
  mkdir -p "$dir"
  cp "$archive" "$dir/$(basename "$archive")"
  cp "$sha" "$dir/$(basename "$sha")"
  cp "$PROJECT_ROOT/install.sh" "$dir/install.sh"
}

# stage_old_releases <served-root> <target> — v0.50.0 and v0.49.1 (broken:
# predates `ensure`), repackaged under this branch's install.sh. Prints
# nothing; also leaves v0.50.0's OWN install.sh at
# "$(cache_root)/fetched/$OLD_TAG/$target/install.sh" for the
# old-client-upgrade scenario.
stage_old_releases() {
  local root=$1 target=$2 dir archive
  archive="comemory-$target.tar.xz"
  dir="$(fetch_real_release "$OLD_TAG" "$target")"
  stage_tag "$root" "$OLD_TAG" "$dir/$archive" "$dir/$archive.sha256"
  dir="$(fetch_real_release "$BROKEN_TAG" "$target")"
  stage_tag "$root" "$BROKEN_TAG" "$dir/$archive" "$dir/$archive.sha256"
}

# old_release_own_install_sh <target> — v0.50.0's real, historical
# install.sh (predates this issue's ensure/rollback contract).
old_release_own_install_sh() { printf '%s/install.sh' "$(fetch_real_release "$OLD_TAG" "$1")"; }

# lifecycle_tag <suffix> — `vX.Y.(Z+1)-lifecycle.<suffix>` from Cargo.toml.
lifecycle_tag() {
  local base major_minor patch
  base="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$PROJECT_ROOT/Cargo.toml" | head -1)"
  major_minor="${base%.*}"
  patch=$((${base##*.} + 1))
  printf 'v%s.%s-lifecycle.%s' "$major_minor" "$patch" "$1"
}

# build_lifecycle <suffix> <target-dir> — a scratch `git worktree` at HEAD
# with Cargo.toml's version rewritten, `cargo update -p comemory --offline`,
# then `cargo build --profile release-quick` sharing `target-dir` so
# lifecycle.2 reuses lifecycle.1's dependency compilation. Cached by HEAD sha
# + suffix. Prints `<bin-path>\t<tag>`.
build_lifecycle() {
  local suffix=$1 target_dir=$2 sha tag version cache bin
  sha="$(git -C "$PROJECT_ROOT" rev-parse HEAD)"
  tag="$(lifecycle_tag "$suffix")"
  version="${tag#v}"
  cache="$(cache_root)/lifecycle/$sha/$suffix"
  bin="$cache/comemory"
  if [ ! -x "$bin" ]; then
    mkdir -p "$cache"
    local scratch ok=1
    scratch="$(mktemp -d)/wt"
    git -C "$PROJECT_ROOT" worktree add --detach "$scratch" HEAD >&2 || ok=0
    # Only the [package] table's own `version = "…"` (the first such line);
    # a blanket sed also rewrites every `[dependencies.*]` table's pinned
    # `version = "…"` key, breaking dependency resolution.
    if [ "$ok" -eq 1 ]; then
      awk -v v="$version" '
        !done && /^version = "/ { print "version = \"" v "\""; done = 1; next }
        { print }
      ' "$scratch/Cargo.toml" >"$scratch/Cargo.toml.new" \
        && mv "$scratch/Cargo.toml.new" "$scratch/Cargo.toml" || ok=0
    fi
    if [ "$ok" -eq 1 ]; then
      (cd "$scratch" && cargo update -p comemory --offline) >&2 || ok=0
    fi
    if [ "$ok" -eq 1 ]; then
      (cd "$scratch" && CARGO_TARGET_DIR="$target_dir" cargo build --profile release-quick --locked) >&2 || ok=0
    fi
    if [ "$ok" -eq 1 ]; then
      cp "$target_dir/release-quick/comemory" "$bin" || ok=0
    fi
    git -C "$PROJECT_ROOT" worktree remove --force "$scratch" >&2 || true
    if [ "$ok" -ne 1 ]; then
      printf 'build_lifecycle %s failed\n' "$suffix" >&2
      return 1
    fi
  fi
  # A trailing newline matters: without one, `read` hits EOF mid-line and
  # returns failure even after filling both fields, which aborts the caller
  # under `set -e`.
  printf '%s\t%s\n' "$bin" "$tag"
}

# package_lifecycle_tag <served-root> <target> <bin-path> <tag> — package a
# built lifecycle binary and stage it under `served-root/<tag>/`.
package_lifecycle_tag() {
  local root=$1 target=$2 bin=$3 tag=$4 dest
  dest="$(mktemp -d)"
  tar_xz_package "$bin" "$target" "$dest"
  stage_tag "$root" "$tag" "$dest/comemory-$target.tar.xz" "$dest/comemory-$target.tar.xz.sha256"
  rm -rf "$dest"
}
