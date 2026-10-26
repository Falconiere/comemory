#!/usr/bin/env bash
# install.sh's libc gate on real distro images. Each case pipes the working
# tree's install.sh into a fresh container and checks the gate's verdict:
# musl hosts (Alpine, with gcompat, and with gcompat but no getconf) are
# refused before anything is downloaded; glibc hosts of any version, including
# one that also carries Debian's `musl` package (with or without getconf), get
# past it — which glibc is new enough is the downloaded binary's --version
# run's call, not the gate's. No case downloads a release:
# COMEMORY_RELEASES_URL points at a closed port, so a host the gate lets
# through stops at "could not resolve the latest release".
#
#   bash scripts/test-install-libc.sh
#
# Needs a reachable docker daemon, and network for image pulls and the
# apt/apk setup steps. A missing daemon, or a case whose image or packages
# could not be fetched, fails under $CI and otherwise prints a loud [warn]
# and does not count as a failure, like the other tool-dependent gates.
# Prints one OK / FAIL line per case and exits 1 on any failure.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"
NAME=test-install-libc

# skipped <why> — the gate did not run: fatal in CI, a loud warning locally.
skipped() {
  [[ -z "${CI:-}" ]] || die "$NAME" "$1 (required in CI)"
  printf '%s[%s]%s %s[warn]%s %s\n' "$C_DIM" "$NAME" "$C_RST" "$C_YLW" "$C_RST" "$1" >&2
}

if ! docker info >/dev/null 2>&1; then
  skipped "the libc gate test DID NOT RUN — no reachable docker daemon"
  exit 0
fi

PASSED="could not resolve the latest release"
MUSL="this is a musl system"
NO_GETCONF='rm /usr/bin/getconf'
# A pocket whose index fails to verify must not sink the case: musl comes from
# the release pocket. Each setup then proves its premise is in place.
APT_MUSL='{ apt-get update -qq || true; } && apt-get install -y -qq musl && ls /lib/ld-musl-*'
APK_GCOMPAT='apk add --no-cache gcompat libc6-compat && ls /lib/ld-linux-*'
failed=0

# run_case <name> <image> <setup shell> <want>... — a want of `!text` must be
# absent. A refusal message alone proves the gate ran before the release was
# resolved: resolution always fails against the closed port, and would have
# died first. Exit 125 (docker could not pull or start the image) and 99 (the
# setup step failed) mean the case never reached install.sh.
run_case() {
  local name="$1" image="$2" setup="$3" out want rc=0
  shift 3
  out="$(docker run --rm -i -e NO_COLOR=1 -e COMEMORY_RELEASES_URL=http://127.0.0.1:9 \
    "$image" sh -c 'cat > /tmp/install.sh
      { eval "$1"; } > /tmp/setup.log 2>&1 || { echo "setup failed:"; cat /tmp/setup.log; exit 99; }
      sh /tmp/install.sh --dir /tmp/bin --no-modify-path --no-completions </dev/null' \
    sh "$setup" < "$PROJECT_ROOT/install.sh" 2>&1)" || rc=$?
  if [[ "$rc" == 99 || "$rc" == 125 ]]; then
    skipped "$name DID NOT RUN — could not fetch its image or packages (exit $rc)"
    printf '%s\n' "$out" | tail -n 5 | sed 's/^/    /' >&2
    return
  fi
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

run_case debian-bullseye-glibc-2.31 debian:bullseye-slim true ", glibc 2.31 (" "$PASSED"
run_case rocky-9-glibc-2.34 rockylinux:9 true ", glibc 2.34 (" "$PASSED"
run_case ubuntu-22.04-glibc-2.35 ubuntu:22.04 true ", glibc 2.35 (" "$PASSED"
run_case ubuntu-24.04-with-musl-package ubuntu:24.04 "$APT_MUSL" ", glibc 2.39 (" "$PASSED"
run_case ubuntu-24.04-with-musl-package-no-getconf ubuntu:24.04 "$APT_MUSL && $NO_GETCONF" \
  "$PASSED" "!$MUSL"
run_case alpine alpine:3.20 true "$MUSL"
run_case alpine-gcompat alpine:3.20 "$APK_GCOMPAT" "$MUSL"
run_case alpine-gcompat-no-getconf alpine:3.20 "$APK_GCOMPAT && $NO_GETCONF" "$MUSL"

exit "$failed"
