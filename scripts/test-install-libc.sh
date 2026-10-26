#!/usr/bin/env bash
# install.sh's libc gate on real distro images. Each case pipes the working
# tree's install.sh into a fresh container and checks the gate's verdict:
# musl hosts (Alpine, with gcompat, and with gcompat but no getconf) and
# glibc below the GLIBC_MIN floor are refused before anything is downloaded;
# glibc at or above the floor, and a glibc host that also carries Debian's
# `musl` package (with or without getconf), get past it. No case downloads a
# release: COMEMORY_RELEASES_URL points at a closed port, so a host the gate
# lets through stops at "could not resolve the latest release".
#
# The floor itself is held against the latest release: it must equal the
# newest GLIBC_ symbol version its Linux binaries need. Higher refuses hosts
# they run on; lower lets through hosts whose --version run then fails.
#
#   bash scripts/test-install-libc.sh
#
# Needs a reachable docker daemon, and network for the apt/apk setup steps
# and the release download. Without docker it fails under $CI and otherwise
# prints a loud [warn] and exits 0, like the other tool-dependent gates.
# Prints one OK / FAIL line per case and exits 1 on any failure.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"
NAME=test-install-libc

if ! docker info >/dev/null 2>&1; then
  why="the libc gate test DID NOT RUN — no reachable docker daemon"
  [[ -z "${CI:-}" ]] || die "$NAME" "$why (required in CI)"
  printf '%s[%s]%s %s[warn]%s %s\n' "$C_DIM" "$NAME" "$C_RST" "$C_YLW" "$C_RST" "$why" >&2
  exit 0
fi

FLOOR="$(sed -n 's/^GLIBC_MIN=//p' "$PROJECT_ROOT/install.sh")"
[[ "$FLOOR" =~ ^2\.[0-9]+$ ]] || die "$NAME" "no GLIBC_MIN=2.N line in install.sh"
PASSED="could not resolve the latest release"
MUSL="this is a musl system"
NO_GETCONF='rm /usr/bin/getconf'
APT_MUSL='apt-get update -qq && apt-get install -y -qq musl'
APK_GCOMPAT='apk add --no-cache gcompat libc6-compat'
failed=0

# run_case <name> <image> <setup shell> <want>... — a want of `!text` must be
# absent. A refusal message alone proves the gate ran before the release was
# resolved: resolution always fails against the closed port, and would have
# died first.
run_case() {
  local name="$1" image="$2" setup="$3" out want
  shift 3
  out="$(docker run --rm -i -e NO_COLOR=1 -e COMEMORY_RELEASES_URL=http://127.0.0.1:9 \
    "$image" sh -c 'cat > /tmp/install.sh
      { eval "$1"; } > /tmp/setup.log 2>&1 || { echo "setup failed:"; cat /tmp/setup.log; exit 99; }
      sh /tmp/install.sh --dir /tmp/bin --no-modify-path --no-completions </dev/null' \
    sh "$setup" < "$PROJECT_ROOT/install.sh" 2>&1)" || true
  for want in "$@"; do
    case "$want" in
      !*) [[ "$out" != *"${want#!}"* ]] && continue ;;
      *) [[ "$out" == *"$want"* ]] && continue ;;
    esac
    log_err "$NAME" "$name: wanted '$want' in:"
    printf '%s\n' "$out" | sed 's/^/    /' >&2
    failed=1
    return
  done
  log_ok "$NAME" "$name"
}

# The newest GLIBC_ version each latest-release Linux binary references.
check_floor() {
  local out
  out="$(docker run --rm alpine:3.20 sh -c 'apk add -q --no-cache curl xz >/dev/null || exit 99
    for t in x86_64 aarch64; do
      d="comemory-$t-unknown-linux-gnu"
      v="$(curl -fsSL "$1/$d.tar.xz" | xz -dc | tar -xO "$d/comemory" \
        | grep -ao "GLIBC_2\.[0-9]*" | sed "s/GLIBC_//" | sort -t. -k2 -n | tail -n 1)"
      echo "$t ${v:-unreadable}"
    done' sh "https://github.com/Falconiere/comemory/releases/latest/download" 2>&1)" || true
  if [[ "$out" == "x86_64 $FLOOR"$'\n'"aarch64 $FLOOR" ]]; then
    log_ok "$NAME" "GLIBC_MIN $FLOOR matches the latest release's binaries"
  else
    log_err "$NAME" "GLIBC_MIN $FLOOR in install.sh, but the latest release needs:"
    printf '%s\n' "$out" | sed 's/^/    /' >&2
    failed=1
  fi
}

check_floor
run_case rocky-9-glibc-2.34 rockylinux:9 true ", glibc 2.34 (" "$PASSED"
run_case ubuntu-22.04-glibc-2.35 ubuntu:22.04 true ", glibc 2.35 (" "$PASSED"
run_case ubuntu-24.04-with-musl-package ubuntu:24.04 "$APT_MUSL" ", glibc 2.39 (" "$PASSED"
run_case ubuntu-24.04-with-musl-package-no-getconf ubuntu:24.04 "$APT_MUSL && $NO_GETCONF" \
  "$PASSED" "!$MUSL"
run_case debian-bullseye-glibc-2.31 debian:bullseye-slim true \
  "glibc 2.31 is older than $FLOOR"
run_case alpine alpine:3.20 true "$MUSL"
run_case alpine-gcompat alpine:3.20 "$APK_GCOMPAT" "$MUSL"
run_case alpine-gcompat-no-getconf alpine:3.20 "$APK_GCOMPAT && $NO_GETCONF" "$MUSL"

exit "$failed"
