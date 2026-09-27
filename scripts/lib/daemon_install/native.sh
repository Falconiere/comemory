#!/usr/bin/env bash
# Native-supervisor helpers for scripts/test-daemon-install.sh --native
# (AC-8): the OS this host's coordinators should be running under, and how
# many of its per-data-directory unit files exist for a data dir. Sourced,
# never executed. Never touches the developer's own launchd/systemd — this
# file is only reached under `--native`, which itself refuses outside CI
# (test-daemon-install.sh's own guard).

set -euo pipefail

# native_expected_supervisor — the `supervisor` string readiness must report
# under `--native` on this OS.
native_expected_supervisor() {
  case "$(uname -s)" in
    Darwin) echo launchd ;;
    Linux) echo systemd ;;
    *) return 1 ;;
  esac
}

# native_unit_glob <home> — where this OS's per-directory unit files for
# comemory live, as a glob.
native_unit_glob() {
  local home=$1
  case "$(uname -s)" in
    Darwin) printf '%s/Library/LaunchAgents/io.comemory.sync.*.plist' "$home" ;;
    Linux) printf '%s/.config/systemd/user/comemory-sync-*.service' "$home" ;;
    *) return 1 ;;
  esac
}

# native_unit_count <home> <canonical-data-dir> — how many of this host's
# comemory unit files name `canonical-data-dir` in their body (a plist's
# `COMEMORY_DATA_DIR` string, a systemd unit's `Environment=` line).
native_unit_count() {
  local home=$1 data_dir=$2 glob count=0 f
  glob="$(native_unit_glob "$home")"
  # shellcheck disable=SC2086 # intentional glob expansion, not word-split
  for f in $glob; do
    [ -f "$f" ] || continue
    grep -qF "$data_dir" "$f" && count=$((count + 1))
  done
  printf '%s' "$count"
}

# require_native_env <step> — refuse `--native` outside a disposable
# environment (D10): a developer's session never runs this.
require_native_env() {
  if [ "${CI:-}" = true ] || [ "${COMEMORY_DISPOSABLE_ENV:-}" = 1 ]; then
    return 0
  fi
  printf '%s: --native refuses outside CI (set CI=true or COMEMORY_DISPOSABLE_ENV=1)\n' "$1" >&2
  return 2
}
