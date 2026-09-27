#!/usr/bin/env bash
# `kill-after-rename` (AC-6(c)) and `rollback` (AC-4): a binary swapped in
# without `ensure` is repaired by the next ordinary command; a release that
# can never become ready is rolled back byte-for-byte, keeping the old
# coordinator's pid. Sourced, never executed.

set -euo pipefail

scenario_kill_after_rename() {
  local root=$1
  local bin=$root/bin/comemory

  run_install "$MODE" "$root" "$root/bin" --version "$OLD_TAG"
  assert "v0.50.0 install exited $RUN_CODE: $RUN_ERR" test "$RUN_CODE" -eq 0 || return 1
  wait_running "$MODE" "$root" "$bin" 30
  assert "v0.50.0 coordinator never verified ready" test $? -eq 0 || return 1
  local pid1
  pid1="$(json_num "$RUN_OUT" pid)"

  cp "$LIFECYCLE1_BIN" "$root/bin/.rename.$$"
  chmod 755 "$root/bin/.rename.$$"
  mv -f "$root/bin/.rename.$$" "$bin"

  wait_replaced "$MODE" "$root" "$bin" "$pid1" "${LIFECYCLE1_TAG#v}" 30
  assert "no v0.50.0 -> lifecycle.1 replacement within 30s" test $? -eq 0 || return 1
  local pid2
  pid2="$(json_num "$RUN_OUT" pid)"

  cp "$LIFECYCLE2_BIN" "$root/bin/.rename2.$$"
  chmod 755 "$root/bin/.rename2.$$"
  mv -f "$root/bin/.rename2.$$" "$bin"

  wait_replaced "$MODE" "$root" "$bin" "$pid2" "${LIFECYCLE2_TAG#v}" 30
  assert "no lifecycle.1 -> lifecycle.2 replacement within 30s" test $? -eq 0 || return 1
  return 0
}

scenario_rollback() {
  local root=$1
  local bin=$root/bin/comemory

  run_install "$MODE" "$root" "$root/bin" --version "$OLD_TAG"
  assert "v0.50.0 install exited $RUN_CODE: $RUN_ERR" test "$RUN_CODE" -eq 0 || return 1
  wait_running "$MODE" "$root" "$bin" 30
  assert "v0.50.0 coordinator never verified ready" test $? -eq 0 || return 1
  local old_pid old_sha
  old_pid="$(json_num "$RUN_OUT" pid)"
  old_sha="$(sha256_file "$bin")"

  run_install "$MODE" "$root" "$root/bin" --version "$BROKEN_TAG"
  assert "v0.49.1 install exited $RUN_CODE, want 69: $RUN_ERR" test "$RUN_CODE" -eq 69 || return 1
  case "$RUN_ERR" in
    *"rolled back"*) ;;
    *)
      scen_fail "stderr lacks 'rolled back': $RUN_ERR"
      return 1
      ;;
  esac
  case "$RUN_OUT" in
    *installed*)
      scen_fail "stdout claims an install headline: $RUN_OUT"
      return 1
      ;;
  esac

  assert "binary changed across the rollback" test "$(sha256_file "$bin")" = "$old_sha" || return 1
  wait_running "$MODE" "$root" "$bin" 30
  assert "the restored coordinator never verified ready" test $? -eq 0 || return 1
  assert "restored pid $(json_num "$RUN_OUT" pid) != old pid $old_pid" \
    test "$(json_num "$RUN_OUT" pid)" = "$old_pid" || return 1
  return 0
}
