#!/usr/bin/env bash
# Shared isolation, JSON and process helpers for scripts/test-daemon-install.sh
# and its scenario/binaries libraries. Sourced, never executed; every
# function here is real-process only — no stub answers `ensure`.

set -euo pipefail

# Whether any scenario has failed so far (the caller exits 1 if so). Read by
# scripts/test-daemon-install.sh, which sources this file.
DAEMON_INSTALL_FAILED=0

pass() { printf 'PASS %s\n' "$1"; }
fail() {
  printf 'FAIL %s: %s\n' "$1" "$2"
  # shellcheck disable=SC2034 # read by scripts/test-daemon-install.sh
  DAEMON_INSTALL_FAILED=1
}

# scen_fail <reason> — a scenario body's one way to report an assertion
# miss: set the reason `run_scenario` prints and return 1 without aborting
# the harness (every scenario call sits in an `if`, which bash's own
# errexit-suspension-in-a-condition rule keeps from killing the script).
SCEN_REASON=""
scen_fail() {
  # shellcheck disable=SC2034 # read by run_scenario in scenarios.sh
  SCEN_REASON="$1"
  return 1
}

# assert <message> <test-argv...> — `"$message"` becomes the scenario's
# failure reason unless `test "${@:2}"` holds. Callers always chain
# `|| return 1`: `assert` alone only fails itself, since bash's errexit
# suspension (this call graph runs inside `run_scenario`'s `if`) never
# returns out of the caller on its own.
assert() {
  local msg=$1
  shift
  if "$@"; then
    return 0
  fi
  scen_fail "$msg"
}

# ------------------------------------------------------------- json ----
# json_field <json> <key> — a `"key":"value"` string, D2's whitespace-
# tolerant sed (shared with install.sh's own `json_str`), but the FIRST
# occurrence: readiness nests several documents with same-named keys
# (`state` at the top level, again under `auth`, again under `sync`;
# `supervisor` at the top level and again under `daemon`), and a plain
# greedy `sed` match finds the LAST one instead.
json_field() {
  printf '%s' "$1" | grep -o "\"$2\":[[:space:]]*\"[^\"]*\"" | head -1 \
    | sed -n 's/.*"\([^"]*\)"$/\1/p'
}
# json_num <json> <key> — a bare `"key":123` number, first occurrence.
json_num() {
  printf '%s' "$1" | grep -o "\"$2\":[[:space:]]*[0-9][0-9]*" | head -1 \
    | sed -n 's/.*:[[:space:]]*//p'
}
# json_bool <json> <key> — a bare `"key":true|false`, first occurrence.
json_bool() {
  printf '%s' "$1" | grep -o "\"$2\":[[:space:]]*\(true\|false\)" | head -1 \
    | sed -n 's/.*:[[:space:]]*//p'
}

# --------------------------------------------------------- platform ----
# host_target — the cargo-dist target triple for this host, or nonzero when
# comemory publishes no build for it (mirrors release_server.rs::host_target).
host_target() {
  case "$(uname -s):$(uname -m)" in
    Darwin:arm64) echo aarch64-apple-darwin ;;
    Linux:x86_64) echo x86_64-unknown-linux-gnu ;;
    Linux:aarch64 | Linux:arm64) echo aarch64-unknown-linux-gnu ;;
    *) return 1 ;;
  esac
}

# --------------------------------------------------------- isolation ----
# new_root — a fresh private root under a short /tmp path.
new_root() { mktemp -d /tmp/cdi.XXXXXX; }

# init_root <root> — this scenario's directories: h (HOME), d (data dir),
# t (TMPDIR), bin (install target).
init_root() {
  local root=$1
  mkdir -p "$root/h" "$root/d" "$root/t" "$root/bin"
}

# --------------------------------------------------- running comemory ----
# run_bin <mode> <root> <bin> -- <args...> — `mode` is `headless` (private
# HOME, forced `process` supervisor, no session bus) or `native` (the real
# HOME, so launchd/systemd user managers see the unit; data dir stays
# private). Sets RUN_CODE/RUN_OUT/RUN_ERR; never propagates the exit code
# itself (`set -e` would abort the harness on the first expected failure).
run_bin() {
  local mode=$1 root=$2 bin=$3
  shift 3
  [ "${1:-}" != "--" ] || shift
  local out err home_dir
  out="$(mktemp)"
  err="$(mktemp)"
  home_dir="$root/h"
  # Every `-u` precedes every assignment (see install_helpers.sh). Native
  # mode keeps the session bus: systemd --user needs it (D10).
  local -a envs=(-u COMEMORY_API_KEY -u COMEMORY_SYNC_DAEMON)
  if [ "$mode" = native ]; then
    home_dir="${DAEMON_INSTALL_NATIVE_HOME:-$HOME}"
  else
    envs+=(-u XDG_RUNTIME_DIR -u DBUS_SESSION_BUS_ADDRESS COMEMORY_DAEMON_SUPERVISOR=process)
  fi
  envs+=(HOME="$home_dir" TMPDIR="$root/t" COMEMORY_DATA_DIR="$root/d" COMEMORY_INDEXING_AUTO_REINDEX=off)
  if env "${envs[@]}" "$bin" "$@" >"$out" 2>"$err"; then
    RUN_CODE=0
  else
    RUN_CODE=$?
  fi
  RUN_OUT="$(cat "$out")"
  # shellcheck disable=SC2034 # read by scenario files for failure messages
  RUN_ERR="$(cat "$err")"
  rm -f "$out" "$err"
}

# wait_running <mode> <root> <bin> [timeout] — poll `sync daemon status
# --json` (read-only; starts nothing) until it reports a running
# coordinator. Sets RUN_OUT to that JSON; nonzero on timeout.
wait_running() {
  local mode=$1 root=$2 bin=$3 timeout=${4:-30}
  local deadline=$((SECONDS + timeout))
  while :; do
    run_bin "$mode" "$root" "$bin" -- sync daemon status --json
    if [ "$RUN_CODE" -eq 0 ] && [ "$(json_field "$RUN_OUT" state)" = running ]; then
      return 0
    fi
    [ "$SECONDS" -lt "$deadline" ] || return 1
    sleep 0.3
  done
}

# wait_replaced <mode> <root> <bin> <old-pid> <want-version> [timeout] —
# repeatedly runs an ordinary command (`stats`, which preflight gates) to
# nudge D3c's replacement rule, then checks `status` for a NEW pid at the
# wanted version (AC-6(c): a kill-after-rename recovers on the next
# command). Leaves the winning `status --json` in RUN_OUT.
wait_replaced() {
  local mode=$1 root=$2 bin=$3 old_pid=$4 want_version=$5 timeout=${6:-30}
  local deadline=$((SECONDS + timeout))
  while :; do
    run_bin "$mode" "$root" "$bin" -- stats --json
    run_bin "$mode" "$root" "$bin" -- sync daemon status --json
    if [ "$(json_field "$RUN_OUT" state)" = running ]; then
      local pid version
      pid="$(json_num "$RUN_OUT" pid)"
      version="$(json_field "$RUN_OUT" version)"
      if [ -n "$pid" ] && [ "$pid" != "$old_pid" ] && [ "$version" = "$want_version" ]; then
        return 0
      fi
    fi
    [ "$SECONDS" -lt "$deadline" ] || return 1
    sleep 0.3
  done
}

# --------------------------------------------------------- processes ----
# coordinator_pids <canonical-data-dir> — live `… --data-dir <dir> sync
# daemon run` pids, the same `ps` needle daemon_support.rs uses.
coordinator_pids() {
  # The needle travels via an exported ENV var, never `awk -v` or an argv
  # word: either would put the literal needle text into `awk`'s own command
  # line, which the very same `ps` snapshot can catch mid-pipeline — a
  # false "coordinator" match on awk's own pid (proven live: it happens on
  # every call, not just rarely).
  export CDI_COORDINATOR_NEEDLE="--data-dir $1 sync daemon run"
  ps -axo pid=,command= | awk 'index($0, ENVIRON["CDI_COORDINATOR_NEEDLE"]) { print $1 }'
}

# file_id <path> — `<dev>:<ino>`, the same identity readiness's
# `binary_file` reports (`MetadataExt::{dev,ino}` on the Rust side).
file_id() {
  if stat -f '%d:%i' "$1" >/dev/null 2>&1; then
    stat -f '%d:%i' "$1"
  else
    stat -c '%d:%i' "$1"
  fi
}

# canonical <path> — the physical, symlink-resolved path (what readiness's
# `binary` reports).
canonical() { (cd -P "$(dirname "$1")" && printf '%s/%s' "$(pwd -P)" "$(basename "$1")"); }

alive() { kill -0 "$1" 2>/dev/null; }

# wait_gone <pid> [timeout] — poll until `pid` is no longer alive.
wait_gone() {
  local pid=$1 timeout=${2:-30} deadline
  deadline=$((SECONDS + timeout))
  while alive "$pid"; do
    [ "$SECONDS" -lt "$deadline" ] || return 1
    sleep 0.2
  done
  return 0
}

# dump_tables <db-path> — a stable `.dump` of the tables an upgrade must
# carry intact: memories, and the outbox/cursor tables that exist
# (`replica_operation`, `replica_cursor`).
dump_tables() {
  printf '.dump memories\n.dump replica_operation\n.dump replica_cursor\n' | sqlite3 "$1"
}

terminate() {
  local pid=$1 deadline
  kill -TERM "$pid" 2>/dev/null || return 0
  deadline=$((SECONDS + 5))
  while alive "$pid" && [ "$SECONDS" -lt "$deadline" ]; do sleep 0.1; done
  alive "$pid" && kill -KILL "$pid" 2>/dev/null || true
}

# wait_one_coordinator <canonical-data-dir> [timeout] — poll until exactly
# one coordinator answers `ps` for this data dir (a stopped-but-not-yet-
# reaped loser can briefly leave two alive). Prints the current pid list;
# 0 once it settles at one, 1 on timeout.
wait_one_coordinator() {
  local data_dir=$1 timeout=${2:-30} deadline count pids
  deadline=$((SECONDS + timeout))
  while :; do
    pids="$(coordinator_pids "$data_dir")"
    count=$(printf '%s\n' "$pids" | grep -c .)
    if [ "$count" -eq 1 ]; then
      printf '%s' "$pids"
      return 0
    fi
    if [ "$SECONDS" -ge "$deadline" ]; then
      printf '%s' "$pids"
      return 1
    fi
    sleep 0.3
  done
}

# stop_root <root> — terminate every coordinator this root's data dir still
# runs (a scenario's cleanup, best effort).
stop_root() {
  local root=$1 canonical pid
  [ -d "$root/d" ] || return 0
  canonical="$(cd -P "$root/d" && pwd -P)"
  for pid in $(coordinator_pids "$canonical"); do terminate "$pid"; done
}
