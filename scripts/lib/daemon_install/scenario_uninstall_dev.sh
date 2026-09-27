#!/usr/bin/env bash
# `uninstall` (AC-7) and `dev-install` (AC-1): uninstall touches only this
# data directory's unit and socket; scripts/dev-install.sh finishes on a
# verified sync daemon and fails honestly under an external supervisor.
# Sourced, never executed.

set -euo pipefail

scenario_uninstall() {
  local root=$1
  local bin=$root/bin/comemory

  run_install "$MODE" "$root" "$root/bin" --version "$LIFECYCLE1_TAG"
  assert "install exited $RUN_CODE: $RUN_ERR" test "$RUN_CODE" -eq 0 || return 1
  wait_running "$MODE" "$root" "$bin" 30
  assert "coordinator never verified ready" test $? -eq 0 || return 1
  local first_pid socket
  first_pid="$(json_num "$RUN_OUT" pid)"
  socket="$(json_field "$RUN_OUT" socket)"

  run_bin "$MODE" "$root" "$bin" -- save "uninstall-scenario memory" --json
  assert "save failed: $RUN_ERR" test "$RUN_CODE" -eq 0 || return 1
  local memory_id
  memory_id="$(json_field "$RUN_OUT" id)"

  local root2
  root2="$(new_root)"
  init_root "$root2"
  run_install "$MODE" "$root2" "$root2/bin" --version "$LIFECYCLE1_TAG"
  assert "second install exited $RUN_CODE: $RUN_ERR" test "$RUN_CODE" -eq 0 || return 1
  wait_running "$MODE" "$root2" "$root2/bin/comemory" 30
  assert "second coordinator never verified ready" test $? -eq 0 || return 1

  run_bin "$MODE" "$root" "$bin" -- sync daemon uninstall --json
  assert "uninstall exited $RUN_CODE: $RUN_ERR" test "$RUN_CODE" -eq 0 || return 1

  wait_gone "$first_pid" 30
  assert "first coordinator still alive after uninstall" test $? -eq 0 || return 1
  assert "socket still present: $socket" test ! -e "$socket" || return 1
  assert "comemory.db missing after uninstall" test -f "$root/d/comemory.db" || return 1

  run_bin "$MODE" "$root" "$bin" -- show "$memory_id" --json
  assert "show failed after uninstall: $RUN_ERR" test "$RUN_CODE" -eq 0 || return 1
  case "$RUN_OUT" in
    *"uninstall-scenario memory"*) ;;
    *)
      scen_fail "saved memory missing after uninstall: $RUN_OUT"
      stop_root "$root2"
      rm -rf "$root2"
      return 1
      ;;
  esac

  run_bin "$MODE" "$root2" "$root2/bin/comemory" -- sync daemon status --json
  local second_ok=0
  [ "$RUN_CODE" -eq 0 ] && [ "$(json_field "$RUN_OUT" state)" = running ] && second_ok=1
  stop_root "$root2"
  rm -rf "$root2"
  assert "second data dir's coordinator did not survive" test "$second_ok" -eq 1 || return 1
  return 0
}

# _dev_install_run <root> <extra-env...> — scripts/dev-install.sh --no-clean
# under a private HOME/CARGO_INSTALL_ROOT (the real CARGO_HOME, so no
# registry re-download) and no completions (the harness never touches a
# shell rc). Sets RUN_CODE/RUN_OUT/RUN_ERR.
_dev_install_run() {
  local root=$1
  shift
  local out err
  out="$(mktemp)"
  err="$(mktemp)"
  local -a envs=(-u XDG_RUNTIME_DIR -u DBUS_SESSION_BUS_ADDRESS -u COMEMORY_API_KEY
    HOME="$root/h" TMPDIR="$root/t" CARGO_HOME="$REAL_CARGO_HOME" RUSTUP_HOME="$REAL_RUSTUP_HOME"
    CARGO_INSTALL_ROOT="$root/cargo" COMEMORY_DATA_DIR="$root/d"
    COMEMORY_INDEXING_AUTO_REINDEX=off "$@")
  if env "${envs[@]}" bash "$PROJECT_ROOT/scripts/dev-install.sh" --no-clean --no-completions \
    >"$out" 2>"$err"; then
    RUN_CODE=0
  else
    RUN_CODE=$?
  fi
  RUN_OUT="$(cat "$out")"
  RUN_ERR="$(cat "$err")"
  rm -f "$out" "$err"
}

scenario_dev_install() {
  local root=$1
  local bin=$root/cargo/bin/comemory

  _dev_install_run "$root" COMEMORY_DAEMON_SUPERVISOR=process
  assert "dev-install.sh exited $RUN_CODE: $RUN_ERR" test "$RUN_CODE" -eq 0 || return 1
  case "$RUN_OUT$RUN_ERR" in
    *"sync daemon ready"*) ;;
    *)
      scen_fail "no 'sync daemon ready' line: $RUN_OUT$RUN_ERR"
      return 1
      ;;
  esac
  assert "no binary at $bin" test -x "$bin" || return 1

  run_bin "$MODE" "$root" "$bin" -- sync daemon status --json
  assert "status failed after dev-install" test "$RUN_CODE" -eq 0 || return 1
  assert "binary $(json_field "$RUN_OUT" binary) != $(canonical "$bin")" \
    test "$(json_field "$RUN_OUT" binary)" = "$(canonical "$bin")" || return 1

  _dev_install_run "$root" COMEMORY_DAEMON_SUPERVISOR=external
  assert "external-supervisor dev-install exited 0, expected a failure" test "$RUN_CODE" -ne 0 || return 1
  case "$RUN_ERR" in
    *"the sync daemon is not ready"*) ;;
    *)
      scen_fail "stderr lacks 'the sync daemon is not ready': $RUN_ERR"
      return 1
      ;;
  esac
  return 0
}
