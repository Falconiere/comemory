#!/usr/bin/env bash
# Real-brew keg upgrade (Falconiere/homebrew-tap#1, F-2). The issue row
# "change Cellar version/opt symlink, then invoke CLI: the service follows
# the stable expected binary, no stale version". Uses real `brew install` /
# `brew upgrade` of a local tap formula serving two real branch builds
# (`lifecycle.1` -> `lifecycle.2`), real launchd/systemd (`--native`) or the
# process supervisor (`--headless`), and the real SQLite corpus.
#
# `HOMEBREW_NO_INSTALL_CLEANUP=1` keeps the old keg, as brew does on macOS,
# so the old coordinator's binary still exists after the upgrade. The first
# ordinary command must then move the daemon onto the newly linked keg, and
# the unit must run the stable `opt` link.
#
#   bash scripts/test-brew-keg-upgrade.sh (--native|--headless) [--new-bin P --next-bin P]
#
# Installs into the real Homebrew prefix, so this refuses unless CI=true or
# COMEMORY_DISPOSABLE_ENV=1. Prints `PASS brew-keg-upgrade` or
# `FAIL brew-keg-upgrade: <why>`.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
LIB="$HERE/lib/daemon_install"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

usage() { echo "usage: $0 (--native|--headless) [--new-bin P --next-bin P]" >&2; exit 2; }
MODE="" NEW_BIN="" NEXT_BIN=""
while [ $# -gt 0 ]; do
  case "$1" in
    --native | --headless) [ -z "$MODE" ] || usage; MODE="${1#--}"; shift ;;
    --new-bin) [ $# -ge 2 ] || usage; NEW_BIN=$2; shift 2 ;;
    --next-bin) [ $# -ge 2 ] || usage; NEXT_BIN=$2; shift 2 ;;
    *) usage ;;
  esac
done
[ -n "$MODE" ] || usage
if { [ -n "$NEW_BIN" ] || [ -n "$NEXT_BIN" ]; } && { [ -z "$NEW_BIN" ] || [ -z "$NEXT_BIN" ]; }; then
  usage
fi
if [ "${CI:-}" != true ] && [ "${COMEMORY_DISPOSABLE_ENV:-}" != 1 ]; then
  echo "test-brew-keg-upgrade: installs into the real Homebrew prefix; refuses outside CI (set CI=true or COMEMORY_DISPOSABLE_ENV=1)" >&2
  exit 2
fi
require_cmd brew
require_cmd sqlite3

# shellcheck source=scripts/lib/daemon_install/common.sh
source "$LIB/common.sh"
# shellcheck source=scripts/lib/daemon_install/native.sh
source "$LIB/native.sh"
# shellcheck source=scripts/lib/daemon_install/binaries.sh
source "$LIB/binaries.sh"

TARGET="$(host_target)" || { echo "no comemory build target for this host" >&2; exit 1; }
if [ -n "$NEW_BIN" ]; then
  L1_BIN=$NEW_BIN L2_BIN=$NEXT_BIN
  L1_TAG="$(lifecycle_tag 1)" L2_TAG="$(lifecycle_tag 2)"
else
  BUILD_DIR="${DAEMON_INSTALL_BUILD_DIR:-$PROJECT_ROOT/target/lifecycle-build}"
  IFS=$'\t' read -r L1_BIN L1_TAG < <(build_lifecycle 1 "$BUILD_DIR")
  IFS=$'\t' read -r L2_BIN L2_TAG < <(build_lifecycle 2 "$BUILD_DIR")
fi

export HOMEBREW_NO_AUTO_UPDATE=1 HOMEBREW_NO_INSTALL_CLEANUP=1 HOMEBREW_NO_ANALYTICS=1 HOMEBREW_NO_ENV_HINTS=1
TAP="comemory-test/keg"
ROOT="$(new_root)"
init_root "$ROOT"
PREFIX="$(brew --prefix)"
OPT_BIN="$PREFIX/opt/comemory/bin/comemory"
LINK_BIN="$PREFIX/bin/comemory"

cleanup() {
  [ ! -x "$OPT_BIN" ] || run_bin "$MODE" "$ROOT" "$OPT_BIN" -- sync daemon uninstall --json
  stop_root "$ROOT"
  brew uninstall --force "$TAP/comemory" >/dev/null 2>&1 || true
  brew untap "$TAP" >/dev/null 2>&1 || true
  rm -rf "$ROOT"
}
trap cleanup EXIT

die_keg() {
  printf 'FAIL brew-keg-upgrade: %s\n' "$1"
  diagnose_root "$ROOT" >&2
  exit 1
}

# write_formula <tag> <binary>: the tap formula serving one real build.
write_formula() {
  local tag=$1 bin=$2 dest
  dest="$ROOT/pkg-$tag"
  tar_xz_package "$bin" "$TARGET" "$dest"
  cat >"$TAP_DIR/Formula/comemory.rb" <<EOF
class Comemory < Formula
  desc "comemory branch build for the Homebrew keg upgrade test"
  homepage "https://github.com/Falconiere/comemory"
  url "file://$dest/comemory-$TARGET.tar.xz"
  version "${tag#v}"
  sha256 "$(sha256_file "$dest/comemory-$TARGET.tar.xz")"

  def install
    bin.install "comemory"
  end
end
EOF
}

# unit_names_opt: every unit for this data dir runs the stable opt link.
unit_names_opt() {
  local glob f found=0
  glob="$(native_unit_glob "$HOME")"
  while IFS= read -r f; do
    grep -qF "$DATA" "$f" || continue
    found=1
    grep -qF "$OPT_BIN" "$f" || return 1
  done < <(find "$(dirname "$glob")" -maxdepth 1 -type f -name "$(basename "$glob")")
  [ "$found" -eq 1 ]
}

brew tap-new --no-git "$TAP" >/dev/null
TAP_DIR="$(brew --repository "$TAP")"
brew trust --tap "$TAP" >/dev/null

write_formula "$L1_TAG" "$L1_BIN"
brew install "$TAP/comemory" || die_keg "brew install of ${L1_TAG#v} failed"
KEG1="$(canonical "$OPT_BIN")"
DATA="$(cd -P "$ROOT/d" && pwd -P)"

run_bin "$MODE" "$ROOT" "$OPT_BIN" -- sync daemon ensure --json
[ "$RUN_CODE" -eq 0 ] || die_keg "ensure on ${L1_TAG#v} exited $RUN_CODE: $RUN_OUT $RUN_ERR"
wait_running "$MODE" "$ROOT" "$OPT_BIN" 30 || die_keg "the ${L1_TAG#v} coordinator never verified ready"
OLD_PID="$(json_num "$RUN_OUT" pid)"
[ "$(json_field "$RUN_OUT" binary)" = "$KEG1" ] || die_keg "coordinator runs $(json_field "$RUN_OUT" binary), not $KEG1"
if [ "$MODE" = native ]; then
  unit_names_opt || die_keg "the unit does not run $OPT_BIN"
fi

run_bin "$MODE" "$ROOT" "$LINK_BIN" -- save "Queued before a Homebrew keg upgrade." --json
[ "$RUN_CODE" -eq 0 ] || die_keg "save failed: $RUN_ERR"
BEFORE="$(dump_tables "$DATA/comemory.db")"

write_formula "$L2_TAG" "$L2_BIN"
brew upgrade "$TAP/comemory" || die_keg "brew upgrade to ${L2_TAG#v} failed"
KEG2="$(canonical "$OPT_BIN")"
[ "$KEG2" != "$KEG1" ] || die_keg "brew upgrade kept the opt link on $KEG1"
[ -x "$KEG1" ] || die_keg "brew removed the old keg; this test needs it kept (HOMEBREW_NO_INSTALL_CLEANUP)"

# The first ordinary commands through the linked binary: preflight must move
# the daemon onto the new keg even though the old keg's file still exists.
wait_replaced "$MODE" "$ROOT" "$LINK_BIN" "$OLD_PID" "${L2_TAG#v}" 60 \
  || die_keg "after brew upgrade, ordinary commands left the daemon on $(json_field "$RUN_OUT" binary) (v$(json_field "$RUN_OUT" version))"
[ "$(json_field "$RUN_OUT" binary)" = "$KEG2" ] || die_keg "coordinator runs $(json_field "$RUN_OUT" binary), not $KEG2"
wait_gone "$OLD_PID" 30 || die_keg "old coordinator $OLD_PID still runs"
wait_one_coordinator "$DATA" 30 >/dev/null || die_keg "want one coordinator, have: $(coordinator_pids "$DATA" | tr '\n' ' ')"
if [ "$MODE" = native ]; then
  unit_names_opt || die_keg "after the upgrade the unit does not run $OPT_BIN"
fi
[ "$(dump_tables "$DATA/comemory.db")" = "$BEFORE" ] || die_keg "the memories/outbox/cursor tables changed across the upgrade"

printf 'PASS brew-keg-upgrade (%s: %s -> %s, old keg kept)\n' "$MODE" "$KEG1" "$KEG2"
