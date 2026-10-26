#!/usr/bin/env bash
# install.sh's libc gate on real distro images. Each case pipes the working
# tree's install.sh into a fresh container and checks the gate's verdict:
# musl hosts (Alpine, with gcompat, and with gcompat but no getconf) and
# glibc below the 2.35 floor are refused before anything is downloaded;
# glibc at or above the floor, and a glibc host that also carries Debian's
# `musl` package (with or without getconf), get past it. No case downloads a
# release: COMEMORY_RELEASES_URL points at a closed port, so a host the gate
# lets through stops at "could not resolve the latest release".
#
#   bash scripts/test-install-libc.sh
#
# Needs docker, and network for the apt/apk setup steps. Without docker it
# prints a loud [warn] and exits 0, like the other tool-dependent gates.
# Prints `PASS <case>` / `FAIL <case>: <why>` and exits 1 on any failure.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

if ! command -v docker >/dev/null 2>&1; then
  echo "[test-install-libc] [warn] the libc gate test DID NOT RUN — docker is not on PATH" >&2
  exit 0
fi

PASSED="could not resolve the latest release"
MUSL="this is a musl system"
NO_GETCONF='rm /usr/bin/getconf'
APT_MUSL='apt-get update -qq && apt-get install -y -qq musl'
APK_GCOMPAT='apk add --no-cache gcompat libc6-compat'
failed=0

# run_case <name> <image> <setup shell> <want>... — a want of `!text` must be
# absent. Every refusal also asserts no `Version` step ran, which proves it
# came before the release was resolved, let alone downloaded.
run_case() {
  local name="$1" image="$2" setup="$3" out want
  shift 3
  out="$(docker run --rm -i -e NO_COLOR=1 -e COMEMORY_RELEASES_URL=http://127.0.0.1:9 \
    "$image" sh -c 'cat > /tmp/install.sh
      { eval "$1"; } > /tmp/setup.log 2>&1 || { echo "setup failed:"; cat /tmp/setup.log; exit 99; }
      sh /tmp/install.sh --dir /tmp/bin --no-modify-path --no-completions </dev/null' \
    sh "$setup" < "$ROOT/install.sh" 2>&1)" || true
  for want in "$@"; do
    case "$want" in
      !*) [[ "$out" != *"${want#!}"* ]] && continue ;;
      *) [[ "$out" == *"$want"* ]] && continue ;;
    esac
    echo "FAIL $name: wanted '$want' in:"
    printf '%s\n' "$out" | sed 's/^/    /'
    failed=1
    return
  done
  echo "PASS $name"
}

run_case ubuntu-22.04-glibc-2.35-floor ubuntu:22.04 true ", glibc 2.35 (" "$PASSED"
run_case ubuntu-24.04-with-musl-package ubuntu:24.04 "$APT_MUSL" ", glibc 2.39 (" "$PASSED"
run_case ubuntu-24.04-with-musl-package-no-getconf ubuntu:24.04 "$APT_MUSL && $NO_GETCONF" \
  "$PASSED" "!$MUSL"
run_case debian-bullseye-glibc-2.31 debian:bullseye-slim true \
  "glibc 2.31 is older than 2.35" "!Version"
run_case alpine alpine:3.20 true "$MUSL" "!Version"
run_case alpine-gcompat alpine:3.20 "$APK_GCOMPAT" "$MUSL" "!Version"
run_case alpine-gcompat-no-getconf alpine:3.20 "$APK_GCOMPAT && $NO_GETCONF" "$MUSL" "!Version"

exit "$failed"
