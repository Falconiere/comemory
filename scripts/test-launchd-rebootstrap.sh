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

BIN="" ROUNDS=25 STUCK_ROUNDS=2
while [ $# -gt 0 ]; do
  case "$1" in
    --bin) [ $# -ge 2 ] || die "launchd-rebootstrap" "--bin needs a path"; BIN=$2; shift 2 ;;
    --rounds) [ $# -ge 2 ] || die "launchd-rebootstrap" "--rounds needs a number"; ROUNDS=$2; shift 2 ;;
    --stuck-rounds) [ $# -ge 2 ] || die "launchd-rebootstrap" "--stuck-rounds needs a number"; STUCK_ROUNDS=$2; shift 2 ;;
    *) die "launchd-rebootstrap" "unknown argument: $1" ;;
  esac
done
[[ "$ROUNDS" =~ ^[1-9][0-9]*$ ]] || die "launchd-rebootstrap" "--rounds must be a positive number"
[[ "$STUCK_ROUNDS" =~ ^[0-9]+$ ]] || die "launchd-rebootstrap" "--stuck-rounds must be a number"
if [ "$(uname -s)" != Darwin ]; then
  log_err "launchd-rebootstrap" "launchd exists only on macOS"
  exit 2
fi

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
  # A stuck round that failed may leave its coordinator stopped.
  for pid in $(coordinator_pids "${DATA:-$ROOT/d}"); do kill -CONT "$pid" 2>/dev/null || true; done
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

# unit_file <canonical-data-dir>: that data dir's plist (exactly one).
unit_file() {
  local data=$1 f found=""
  for f in "$HOME"/Library/LaunchAgents/io.comemory.sync.*.plist; do
    if [ ! -f "$f" ] || ! grep -qF "$data" "$f"; then continue; fi
    [ -z "$found" ] || return 1
    found=$f
  done
  [ -n "$found" ] && printf '%s' "$found"
}

# ensure_round <n> <canonical-data-dir>: ensure, then require launchd
# readiness on the file now at EXE for that data dir.
ensure_round() {
  local n=$1 data=$2 unit
  run_bin "$MODE" "$ROOT" "$EXE" -- sync daemon ensure --json
  [ "$RUN_CODE" -eq 0 ] || die_round "$n" "ensure exited $RUN_CODE: $RUN_OUT $RUN_ERR"
  jq -e --arg f "$(file_id "$EXE")" '
      .ready == true and .supervisor == "launchd" and .daemon.binary_file == $f
      and ([.notes[] | select(test("process supervision"))] | length) == 0' <<<"$RUN_OUT" >/dev/null \
    || die_round "$n" "not ready under launchd on the swapped-in file: $RUN_OUT"
  wait_one_coordinator "$data" 30 >/dev/null \
    || die_round "$n" "want one coordinator, have: $(coordinator_pids "$data" | tr '\n' ' ')"
  unit="$(unit_file "$data")" || die_round "$n" "want exactly one unit for $data"
  launchctl print "gui/$(id -u)/$(basename "$unit" .plist)" >/dev/null 2>&1 \
    || die_round "$n" "label $(basename "$unit" .plist) is not loaded"
}

# read_old_pid <round>: OLD_PID = the coordinator running now, from its own status.
read_old_pid() {
  run_bin "$MODE" "$ROOT" "$EXE" -- sync daemon status --json
  OLD_PID="$(jq -r '.daemon.pid // empty' <<<"$RUN_OUT")"
  [ -n "$OLD_PID" ] || die_round "$1" "no running coordinator before the swap: $RUN_OUT"
}

cp "$BIN" "$EXE"
chmod 755 "$EXE"
DATA="$(cd -P "$ROOT/d" && pwd -P)"
ensure_round 0 "$DATA"
for n in $(seq 1 "$ROUNDS"); do
  read_old_pid "$n"
  swap_in
  ensure_round "$n" "$DATA"
  [ "$(jq -r .daemon.pid <<<"$RUN_OUT")" != "$OLD_PID" ] || die_round "$n" "coordinator $OLD_PID was not replaced"
  wait_gone "$OLD_PID" 30 || die_round "$n" "old coordinator $OLD_PID still runs"
done

# Stuck rounds: the old coordinator cannot exit promptly (SIGSTOP, a real
# signal on the real process), so `bootout` returns while launchd is still
# removing the job — it SIGKILLs only after its 20 s ExitTimeOut. The next
# `bootstrap` of the label must wait for that instead of failing into a
# process fallback that loses daemon.lock to the stuck coordinator.
for n in $(seq 1 "$STUCK_ROUNDS"); do
  read_old_pid "stuck-$n"
  swap_in
  kill -STOP "$OLD_PID"
  ensure_round "stuck-$n" "$DATA"
  [ "$(jq -r .daemon.pid <<<"$RUN_OUT")" != "$OLD_PID" ] || die_round "stuck-$n" "coordinator $OLD_PID was not replaced"
  wait_gone "$OLD_PID" 30 || die_round "stuck-$n" "stuck coordinator $OLD_PID still runs"
done
printf 'PASS launchd-rebootstrap (%s evictions and %s stuck-coordinator replacements re-bootstrapped under launchd)\n' "$ROUNDS" "$STUCK_ROUNDS"
