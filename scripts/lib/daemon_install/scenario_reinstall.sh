#!/usr/bin/env bash
# `reinstall`, `relocate`, `race` (AC-6): a same-version reinstall restarts
# the coordinator on the new inode; relocating to a second `--dir` leaves one
# coordinator on the new path; two concurrent installers into one dir leave
# exactly one. Sourced, never executed.

set -euo pipefail

scenario_reinstall() {
  local root=$1
  local bin=$root/bin/comemory

  run_install "$MODE" "$root" "$root/bin" --version "$LIFECYCLE1_TAG"
  assert "first install exited $RUN_CODE: $RUN_ERR" test "$RUN_CODE" -eq 0 || return 1
  wait_running "$MODE" "$root" "$bin" 30
  assert "coordinator never verified ready" test $? -eq 0 || return 1
  local old_pid old_file
  old_pid="$(json_num "$RUN_OUT" pid)"
  old_file="$(json_field "$RUN_OUT" binary_file)"

  run_install "$MODE" "$root" "$root/bin" --version "$LIFECYCLE1_TAG"
  assert "reinstall exited $RUN_CODE: $RUN_ERR" test "$RUN_CODE" -eq 0 || return 1
  wait_running "$MODE" "$root" "$bin" 30
  assert "coordinator never verified ready after reinstall" test $? -eq 0 || return 1
  local new_pid new_file
  new_pid="$(json_num "$RUN_OUT" pid)"
  new_file="$(json_field "$RUN_OUT" binary_file)"

  assert "pid unchanged across reinstall ($new_pid)" test "$new_pid" != "$old_pid" || return 1
  assert "binary_file $new_file != $(file_id "$bin")" test "$new_file" = "$(file_id "$bin")" || return 1
  assert "binary_file unchanged ($new_file == $old_file)" test "$new_file" != "$old_file" || return 1
  return 0
}

scenario_relocate() {
  local root=$1
  local first=$root/bin second=$root/bin2

  run_install "$MODE" "$root" "$first" --version "$LIFECYCLE1_TAG"
  assert "first install exited $RUN_CODE: $RUN_ERR" test "$RUN_CODE" -eq 0 || return 1
  wait_running "$MODE" "$root" "$first/comemory" 30
  assert "coordinator never verified ready" test $? -eq 0 || return 1
  local old_pid
  old_pid="$(json_num "$RUN_OUT" pid)"

  run_install "$MODE" "$root" "$second" --version "$LIFECYCLE1_TAG"
  assert "relocated install exited $RUN_CODE: $RUN_ERR" test "$RUN_CODE" -eq 0 || return 1
  wait_running "$MODE" "$root" "$second/comemory" 30
  assert "relocated coordinator never verified ready" test $? -eq 0 || return 1
  local now
  now=$RUN_OUT

  assert "binary $(json_field "$now" binary) != $(canonical "$second/comemory")" \
    test "$(json_field "$now" binary)" = "$(canonical "$second/comemory")" || return 1
  assert "old pid $old_pid still alive" test "$(json_num "$now" pid)" != "$old_pid" || return 1

  local canonical_data pids
  canonical_data="$(cd -P "$root/d" && pwd -P)"
  pids="$(wait_one_coordinator "$canonical_data" 30)"
  assert "exactly one coordinator, got: $pids" test $? -eq 0 || return 1
  return 0
}

scenario_race() {
  local root=$1
  local bin=$root/bin/comemory
  local out1 out2 code1 code2

  out1="$(mktemp)"
  out2="$(mktemp)"
  (
    run_install "$MODE" "$root" "$root/bin" --version "$LIFECYCLE1_TAG"
    printf '%s\n%s\n' "$RUN_CODE" "$RUN_ERR" >"$out1"
  ) &
  local p1=$!
  (
    run_install "$MODE" "$root" "$root/bin" --version "$LIFECYCLE1_TAG"
    printf '%s\n%s\n' "$RUN_CODE" "$RUN_ERR" >"$out2"
  ) &
  local p2=$!
  wait "$p1"
  wait "$p2"
  code1="$(head -1 "$out1")"
  code2="$(head -1 "$out2")"
  rm -f "$out1" "$out2"

  local one_ok=0
  [ "$code1" = 0 ] && one_ok=$((one_ok + 1))
  [ "$code2" = 0 ] && one_ok=$((one_ok + 1))
  assert "neither racer succeeded ($code1, $code2)" test "$one_ok" -ge 1 || return 1

  wait_running "$MODE" "$root" "$bin" 30
  assert "coordinator never verified ready after the race" test $? -eq 0 || return 1
  local canonical_data pids
  canonical_data="$(cd -P "$root/d" && pwd -P)"
  pids="$(wait_one_coordinator "$canonical_data" 30)"
  assert "exactly one coordinator, got: $pids" test $? -eq 0 || return 1
  assert "install lock left behind" test ! -e "$root/bin/.comemory-install.lock" || return 1
  return 0
}
