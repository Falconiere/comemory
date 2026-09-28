#!/usr/bin/env bash
# launchd eviction -> re-bootstrap race (Falconiere/homebrew-tap#1, F-4).
#
# A reinstall swaps a new file in at the coordinator's own path. `ensure` then
# evicts the running coordinator. Its label is still loaded, so the first
# `bootstrap` fails, and `bootout` + `bootstrap` follow. `bootout` returns
# before launchd has removed the job, so an immediate `bootstrap` can fail
# with "5: Input/output error". The process fallback then loses daemon.lock to
# the dying coordinator, and `ensure` exits 69.
#
# This drives that exact path ROUNDS times (default 25) against real launchd
# with a real branch build. Every round must end ready under launchd, on the
# swapped-in file, with one coordinator, one loaded label and no process
# fallback.
#
#   bash scripts/test-launchd-rebootstrap.sh [--bin PATH] [--rounds N]
#
# macOS only, and it refuses unless CI=true or COMEMORY_DISPOSABLE_ENV=1
# (real launchd). Prints `PASS launchd-rebootstrap` or
# `FAIL launchd-rebootstrap: <why>`.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
LIB="$HERE/lib/daemon_install"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

BIN="" ROUNDS=25
while [ $# -gt 0 ]; do
  case "$1" in
    --bin) [ $# -ge 2 ] || die "launchd-rebootstrap" "--bin needs a path"; BIN=$2; shift 2 ;;
    --rounds) [ $# -ge 2 ] || die "launchd-rebootstrap" "--rounds needs a number"; ROUNDS=$2; shift 2 ;;
    *) die "launchd-rebootstrap" "unknown argument: $1" ;;
  esac
done
[[ "$ROUNDS" =~ ^[1-9][0-9]*$ ]] || die "launchd-rebootstrap" "--rounds must be a positive number"
[ "$(uname -s)" = Darwin ] || { echo "test-launchd-rebootstrap: launchd exists only on macOS" >&2; exit 2; }

# shellcheck source=scripts/lib/daemon_install/common.sh
source "$LIB/common.sh"
# shellcheck source=scripts/lib/daemon_install/native.sh
source "$LIB/native.sh"
# shellcheck source=scripts/lib/daemon_install/binaries.sh
source "$LIB/binaries.sh"
MODE=native
require_native_env test-launchd-rebootstrap || exit 2
require_cmd jq

if [ -z "$BIN" ]; then
  IFS=$'\t' read -r BIN _ < <(build_lifecycle 1 "${DAEMON_INSTALL_BUILD_DIR:-$PROJECT_ROOT/target/lifecycle-build}")
fi
[ -x "$BIN" ] || die "launchd-rebootstrap" "not an executable: $BIN"

ROOT="$(new_root)"
init_root "$ROOT"
EXE="$ROOT/bin/comemory"
cleanup() {
  [ ! -x "$EXE" ] || run_bin "$MODE" "$ROOT" "$EXE" -- sync daemon uninstall --json
  stop_root "$ROOT"
  rm -rf "$ROOT"
}
trap cleanup EXIT

die_round() {
  printf 'FAIL launchd-rebootstrap: round %s: %s\n' "$1" "$2"
  diagnose_root "$ROOT" >&2
  exit 1
}

# swap_in: a new file at the same path, as a reinstall's atomic rename leaves it.
swap_in() {
  cp "$BIN" "$EXE.new"
  chmod 755 "$EXE.new"
  mv -f "$EXE.new" "$EXE"
}

# unit_file: this data dir's plist (exactly one).
unit_file() {
  local f found=""
  for f in "$HOME"/Library/LaunchAgents/io.comemory.sync.*.plist; do
    if [ ! -f "$f" ] || ! grep -qF "$DATA" "$f"; then continue; fi
    [ -z "$found" ] || return 1
    found=$f
  done
  [ -n "$found" ] && printf '%s' "$found"
}

# ensure_round <n>: ensure, then require launchd readiness on the file now at EXE.
ensure_round() {
  local n=$1 unit
  run_bin "$MODE" "$ROOT" "$EXE" -- sync daemon ensure --json
  [ "$RUN_CODE" -eq 0 ] || die_round "$n" "ensure exited $RUN_CODE: $RUN_OUT $RUN_ERR"
  jq -e --arg f "$(file_id "$EXE")" '
      .ready == true and .supervisor == "launchd" and .daemon.binary_file == $f
      and ([.notes[] | select(test("process supervision"))] | length) == 0' <<<"$RUN_OUT" >/dev/null \
    || die_round "$n" "not ready under launchd on the swapped-in file: $RUN_OUT"
  wait_one_coordinator "$DATA" 30 >/dev/null \
    || die_round "$n" "want one coordinator, have: $(coordinator_pids "$DATA" | tr '\n' ' ')"
  unit="$(unit_file)" || die_round "$n" "want exactly one unit for $DATA"
  launchctl print "gui/$(id -u)/$(basename "$unit" .plist)" >/dev/null 2>&1 \
    || die_round "$n" "label $(basename "$unit" .plist) is not loaded"
}

cp "$BIN" "$EXE"
chmod 755 "$EXE"
DATA="$(cd -P "$ROOT/d" && pwd -P)"
ensure_round 0
for n in $(seq 1 "$ROUNDS"); do
  OLD_PID="$(jq -r .daemon.pid <<<"$RUN_OUT")"
  swap_in
  ensure_round "$n"
  [ "$(jq -r .daemon.pid <<<"$RUN_OUT")" != "$OLD_PID" ] || die_round "$n" "coordinator $OLD_PID was not replaced"
  wait_gone "$OLD_PID" 30 || die_round "$n" "old coordinator $OLD_PID still runs"
done
printf 'PASS launchd-rebootstrap (%s evictions re-bootstrapped under launchd)\n' "$ROUNDS"
