#!/usr/bin/env bash
# Running the real install.sh and the real `comemory upgrade` against the
# local loopback release server, for scripts/test-daemon-install.sh
# scenarios. Sourced, never executed. Expects $SERVER_BASE (the running
# release_server.py's base URL) from the caller.

set -euo pipefail

INSTALL_SH="$PROJECT_ROOT/install.sh"

# _isolated_envs <mode> <root> — the base env-assignment array every helper
# here shares: a private HOME (headless) or the real one (native, so
# launchd/systemd see the unit), a private data dir and TMPDIR, and the
# credential/session variables an installer shell must never leak.
_isolated_envs() {
  local mode=$1 root=$2
  local home_dir=$root/h
  # Every `-u` must precede every `NAME=value` assignment: BSD/GNU `env`
  # both stop parsing options at the first assignment, so a `-u` after one
  # is treated as the command to exec (`env: -u: No such file or
  # directory`), not an unset.
  # Native mode keeps the session bus: systemd --user needs it (D10).
  local -a envs=(-u COMEMORY_API_KEY -u COMEMORY_SYNC_DAEMON -u COMEMORY_INSTALL_DIR
    -u COMEMORY_VERSION -u COMEMORY_NO_MODIFY_PATH)
  if [ "$mode" = native ]; then
    home_dir="${DAEMON_INSTALL_NATIVE_HOME:-$HOME}"
  else
    envs+=(-u XDG_RUNTIME_DIR -u DBUS_SESSION_BUS_ADDRESS COMEMORY_DAEMON_SUPERVISOR=process)
  fi
  envs+=(HOME="$home_dir" TMPDIR="$root/t" COMEMORY_DATA_DIR="$root/d" COMEMORY_INDEXING_AUTO_REINDEX=off)
  ISOLATED_ENVS=("${envs[@]}")
}

# run_install <mode> <root> <dir> <args...> — a specific install.sh (this
# branch's by default) with `--dir <dir> --no-modify-path <args>` against
# $SERVER_BASE. Sets RUN_CODE/RUN_OUT/RUN_ERR. `INSTALL_SH_OVERRIDE`, when
# set, replaces the script run (v0.50.0's own historical copy).
run_install() {
  local mode=$1 root=$2 dir=$3 script=${INSTALL_SH_OVERRIDE:-$INSTALL_SH}
  shift 3
  local out err
  out="$(mktemp)"
  err="$(mktemp)"
  _isolated_envs "$mode" "$root"
  ISOLATED_ENVS+=(COMEMORY_RELEASES_URL="$SERVER_BASE" NO_COLOR=1 SHELL=/bin/sh)
  if env "${ISOLATED_ENVS[@]}" sh "$script" --dir "$dir" --no-modify-path "$@" >"$out" 2>"$err"; then
    RUN_CODE=0
  else
    RUN_CODE=$?
  fi
  RUN_OUT="$(cat "$out")"
  RUN_ERR="$(cat "$err")"
  rm -f "$out" "$err"
}

# run_upgrade <mode> <root> <bin> <args...> — `<bin> --json upgrade <args>`
# against $SERVER_BASE. Sets RUN_CODE/RUN_OUT/RUN_ERR.
run_upgrade() {
  local mode=$1 root=$2 bin=$3
  shift 3
  local out err
  out="$(mktemp)"
  err="$(mktemp)"
  _isolated_envs "$mode" "$root"
  ISOLATED_ENVS+=(COMEMORY_RELEASES_URL="$SERVER_BASE")
  if env "${ISOLATED_ENVS[@]}" "$bin" --json upgrade "$@" >"$out" 2>"$err"; then
    RUN_CODE=0
  else
    # shellcheck disable=SC2034 # read by scenario_*.sh after this call
    RUN_CODE=$?
  fi
  # shellcheck disable=SC2034 # read by scenario_*.sh after this call
  RUN_OUT="$(cat "$out")"
  # shellcheck disable=SC2034 # read by scenario_*.sh after this call
  RUN_ERR="$(cat "$err")"
  rm -f "$out" "$err"
}
