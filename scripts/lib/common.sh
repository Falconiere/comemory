#!/usr/bin/env bash
# Shared helpers for scripts/ — sourced, not executed.
# Provides: PROJECT_ROOT, log_info, log_err, log_ok, die, run_cargo, require_cmd

set -euo pipefail

PROJECT_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
export PROJECT_ROOT

if [[ -t 1 ]]; then
  C_RED=$'\e[31m'; C_GRN=$'\e[32m'; C_YLW=$'\e[33m'; C_DIM=$'\e[2m'; C_RST=$'\e[0m'
else
  C_RED=""; C_GRN=""; C_YLW=""; C_DIM=""; C_RST=""
fi
# Mark color vars as intentionally exported even if a particular caller only
# uses a subset (shellcheck SC2034 otherwise complains).
export C_RED C_GRN C_YLW C_DIM C_RST

log_info() { printf "%s[%s]%s %s\n" "$C_DIM" "$1" "$C_RST" "$2"; }
log_ok()   { printf "%s[%s] OK%s %s\n" "$C_GRN" "$1" "$C_RST" "${2:-}"; }
log_err()  { printf "%s[%s] FAIL%s %s\n" "$C_RED" "$1" "$C_RST" "$2" >&2; }
die()      { log_err "${1:-script}" "${2:-failed}"; exit 1; }

# Run a cargo command from PROJECT_ROOT.
run_cargo() {
  cd "$PROJECT_ROOT" && cargo "$@"
}

# Fail with an install hint if a required command is missing.
# Usage: require_cmd <command> [install-hint]
require_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "$1" "not found — install: ${2:-$1}"
}
