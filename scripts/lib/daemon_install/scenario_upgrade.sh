#!/usr/bin/env bash
# `old-client-upgrade`, `new-client-upgrade` (AC-2, AC-3, D11): the real
# v0.50.0 binary upgrading itself to lifecycle.1 by running the NEW
# install.sh (the point of the whole design), then lifecycle.1 upgrading
# itself to lifecycle.2 through the new `upgrade` code. Sourced, never
# executed.

set -euo pipefail

scenario_old_client_upgrade() {
  local root=$1
  local bin=$root/bin/comemory db=$root/d/comemory.db

  # v0.50.0's OWN install.sh predates 258: it places the binary but never
  # starts the daemon. The scenario runs v0.50.0's `sync daemon ensure`
  # itself, exactly as I-2 (257) already required before this issue.
  # shellcheck disable=SC2034 # read by run_install in install_helpers.sh
  INSTALL_SH_OVERRIDE="$(old_release_own_install_sh "$TARGET")"
  run_install "$MODE" "$root" "$root/bin" --version "$OLD_TAG"
  unset INSTALL_SH_OVERRIDE
  assert "v0.50.0 install exited $RUN_CODE: $RUN_ERR" test "$RUN_CODE" -eq 0 || return 1

  run_bin "$MODE" "$root" "$bin" -- sync daemon ensure --json
  assert "v0.50.0's sync daemon ensure exited $RUN_CODE: $RUN_ERR" test "$RUN_CODE" -eq 0 || return 1

  wait_running "$MODE" "$root" "$bin" 30
  assert "v0.50.0 coordinator never verified ready" test $? -eq 0 || return 1
  local old_pid
  old_pid="$(json_num "$RUN_OUT" pid)"

  run_bin "$MODE" "$root" "$bin" -- save "lifecycle harness memory" --json
  assert "save on v0.50.0 failed: $RUN_ERR" test "$RUN_CODE" -eq 0 || return 1
  local before
  before="$(dump_tables "$db")"

  # v0.50.0's own `Report` predates the `daemon` field (258's own addition):
  # its "installed" status is all its JSON promises. The new pid/version/
  # identity come from probing the file it just swapped in, below.
  run_upgrade "$MODE" "$root" "$bin" --force --version "$LIFECYCLE1_TAG"
  assert "v0.50.0's upgrade exited $RUN_CODE: $RUN_ERR" test "$RUN_CODE" -eq 0 || return 1

  wait_running "$MODE" "$root" "$bin" 30
  assert "new coordinator never verified ready" test $? -eq 0 || return 1
  assert "new pid == old pid ($old_pid)" test "$(json_num "$RUN_OUT" pid)" != "$old_pid" || return 1
  assert "new version $(json_field "$RUN_OUT" version) != ${LIFECYCLE1_TAG#v}" \
    test "$(json_field "$RUN_OUT" version)" = "${LIFECYCLE1_TAG#v}" || return 1
  wait_gone "$old_pid" 30
  assert "old pid $old_pid still alive" test $? -eq 0 || return 1

  local after
  after="$(dump_tables "$db")"
  assert "the outbox/cursor/memories tables changed across the upgrade" \
    test "$after" = "$before" || return 1
  return 0
}

scenario_new_client_upgrade() {
  local root=$1
  local bin=$root/bin/comemory

  run_install "$MODE" "$root" "$root/bin" --version "$LIFECYCLE1_TAG"
  assert "lifecycle.1 install exited $RUN_CODE: $RUN_ERR" test "$RUN_CODE" -eq 0 || return 1
  wait_running "$MODE" "$root" "$bin" 30
  assert "lifecycle.1 coordinator never verified ready" test $? -eq 0 || return 1
  local old_pid
  old_pid="$(json_num "$RUN_OUT" pid)"

  run_upgrade "$MODE" "$root" "$bin" --force --version "$LIFECYCLE2_TAG"
  assert "lifecycle.1's upgrade exited $RUN_CODE: $RUN_ERR" test "$RUN_CODE" -eq 0 || return 1
  local report=$RUN_OUT
  assert "daemon not ready: $report" test "$(json_bool "$report" ready)" = true || return 1
  assert "new pid == old pid ($old_pid)" test "$(json_num "$report" pid)" != "$old_pid" || return 1
  assert "binary_file $(json_field "$report" binary_file) != $(file_id "$bin")" \
    test "$(json_field "$report" binary_file)" = "$(file_id "$bin")" || return 1

  wait_running "$MODE" "$root" "$bin" 30
  assert "new coordinator never verified ready" test $? -eq 0 || return 1
  assert "new version $(json_field "$RUN_OUT" version) != ${LIFECYCLE2_TAG#v}" \
    test "$(json_field "$RUN_OUT" version)" = "${LIFECYCLE2_TAG#v}" || return 1
  wait_gone "$old_pid" 30
  assert "old pid $old_pid still alive" test $? -eq 0 || return 1
  return 0
}
