#!/usr/bin/env bash
# `fresh` (AC-1, AC-8): a logged-out install of lifecycle.1 via install.sh's
# own `/latest` resolution, ending on a verified coordinator whose identity
# is the installed file. Also the vehicle for `--self-test`. Sourced, never
# executed.

set -euo pipefail

scenario_fresh() {
  local root=$1
  local bin=$root/bin/comemory

  run_install "$MODE" "$root" "$root/bin"
  assert "install.sh exited $RUN_CODE: $RUN_ERR" test "$RUN_CODE" -eq 0 || return 1
  case "$RUN_OUT" in
    *"Sync daemon"*"ready"*) ;;
    *)
      scen_fail "no 'Sync daemon … ready' line in: $RUN_OUT"
      return 1
      ;;
  esac
  assert "no binary at $bin" test -x "$bin" || return 1

  wait_running "$MODE" "$root" "$bin" 30
  assert "coordinator never verified ready" test $? -eq 0 || return 1
  local status=$RUN_OUT

  local want_version=${LIFECYCLE1_TAG#v}
  [ "${DAEMON_INSTALL_SELF_TEST:-0}" != 1 ] || want_version="${want_version}-wrong-on-purpose"
  local got_version got_binary got_file got_super
  got_version="$(json_field "$status" version)"
  got_binary="$(json_field "$status" binary)"
  got_file="$(json_field "$status" binary_file)"
  got_super="$(json_field "$status" supervisor)"

  assert "version $got_version != $want_version" test "$got_version" = "$want_version" || return 1
  assert "binary $got_binary != $(canonical "$bin")" test "$got_binary" = "$(canonical "$bin")" || return 1
  assert "binary_file $got_file != $(file_id "$bin")" test "$got_file" = "$(file_id "$bin")" || return 1
  if [ "$MODE" = native ]; then
    assert "supervisor $got_super != $(native_expected_supervisor)" \
      test "$got_super" = "$(native_expected_supervisor)" || return 1
  else
    assert "supervisor $got_super != process" test "$got_super" = process || return 1
  fi
  return 0
}
