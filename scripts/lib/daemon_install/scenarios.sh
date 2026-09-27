#!/usr/bin/env bash
# The scenario table and its runner for scripts/test-daemon-install.sh
# (AC-8). Sourced, never executed.

set -euo pipefail

# shellcheck disable=SC2034 # read by scripts/test-daemon-install.sh
SCENARIO_NAMES=(fresh old-client-upgrade new-client-upgrade reinstall relocate
  race kill-after-rename rollback uninstall dev-install)

# run_scenario <name> — fresh isolation (a private root; `--native` keeps the
# real HOME), dispatch to `scenario_<name-with-underscores>`, then stop every
# coordinator the scenario started. `--self-test` reports `fresh` under the
# name `self-test` and forces its own version check to mismatch.
run_scenario() {
  local name=$1 fn root report_name=$1
  fn="scenario_$(printf '%s' "$name" | tr '-' '_')"
  if [ "${DAEMON_INSTALL_SELF_TEST:-0}" = 1 ]; then
    report_name="self-test"
  fi
  root="$(new_root)"
  init_root "$root"
  SCEN_REASON="scenario body returned without setting a reason"
  if "$fn" "$root"; then
    pass "$report_name"
  else
    fail "$report_name" "$SCEN_REASON"
  fi
  stop_root "$root"
  rm -rf "$root"
}
