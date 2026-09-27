#!/usr/bin/env bash
# The managed-install lifecycle harness (#258 AC-8): the real install.sh,
# the real `comemory upgrade`, and real published/branch binaries against a
# loopback release server. One `PASS <scenario>` or `FAIL <scenario>: <why>`
# line per scenario; never `SKIP`. See docs/designs/2026-09-26-managed-daemon-install.md.

set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
LIB="$HERE/lib/daemon_install"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

usage() {
  cat <<'EOF'
usage: test-daemon-install.sh (--native|--headless) [options]

  --native                real launchd (macOS) / systemd --user (Linux);
                          refuses outside CI (CI=true or
                          COMEMORY_DISPOSABLE_ENV=1)
  --headless              the process supervisor; safe on a developer
                          machine
  --scenario NAME         run one scenario instead of the whole suite
  --self-test             with --scenario fresh: force a version mismatch
                          and expect a `FAIL self-test` line
  --new-bin P --next-bin P  reuse these as lifecycle.1/.2 instead of
                          building them
EOF
}

MODE=""
SCENARIO=""
SELF_TEST=0
NEW_BIN=""
NEXT_BIN=""
while [ $# -gt 0 ]; do
  case "$1" in
    --native)
      [ -z "$MODE" ] || {
        usage >&2
        exit 2
      }
      MODE=native
      shift
      ;;
    --headless)
      [ -z "$MODE" ] || {
        usage >&2
        exit 2
      }
      MODE=headless
      shift
      ;;
    --scenario)
      [ $# -ge 2 ] || {
        usage >&2
        exit 2
      }
      SCENARIO=$2
      shift 2
      ;;
    --self-test)
      SELF_TEST=1
      shift
      ;;
    --new-bin)
      [ $# -ge 2 ] || {
        usage >&2
        exit 2
      }
      NEW_BIN=$2
      shift 2
      ;;
    --next-bin)
      [ $# -ge 2 ] || {
        usage >&2
        exit 2
      }
      NEXT_BIN=$2
      shift 2
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      usage >&2
      printf 'unknown flag: %s\n' "$1" >&2
      exit 2
      ;;
  esac
done
if [ -z "$MODE" ]; then
  usage >&2
  printf 'exactly one of --native or --headless is required\n' >&2
  exit 2
fi
if { [ -n "$NEW_BIN" ] || [ -n "$NEXT_BIN" ]; } && { [ -z "$NEW_BIN" ] || [ -z "$NEXT_BIN" ]; }; then
  usage >&2
  printf -- '--new-bin and --next-bin must be given together\n' >&2
  exit 2
fi
if [ "$SELF_TEST" -eq 1 ] && [ "$SCENARIO" != fresh ]; then
  usage >&2
  printf -- '--self-test requires --scenario fresh\n' >&2
  exit 2
fi
export DAEMON_INSTALL_SELF_TEST=$SELF_TEST

# shellcheck source=scripts/lib/daemon_install/common.sh
source "$LIB/common.sh"
# shellcheck source=scripts/lib/daemon_install/native.sh
source "$LIB/native.sh"
# shellcheck source=scripts/lib/daemon_install/binaries.sh
source "$LIB/binaries.sh"
# shellcheck source=scripts/lib/daemon_install/install_helpers.sh
source "$LIB/install_helpers.sh"
# shellcheck source=scripts/lib/daemon_install/scenarios.sh
source "$LIB/scenarios.sh"
for f in "$LIB"/scenario_*.sh; do
  # shellcheck source=/dev/null
  source "$f"
done

if [ "$MODE" = native ] && ! require_native_env test-daemon-install; then
  exit 2
fi

if [ -n "$SCENARIO" ]; then
  known=0
  for n in "${SCENARIO_NAMES[@]}"; do [ "$n" != "$SCENARIO" ] || known=1; done
  if [ "$known" -eq 0 ]; then
    printf 'unknown scenario: %s\n' "$SCENARIO" >&2
    exit 2
  fi
fi

TARGET="$(host_target)" || {
  printf 'no published comemory build for this host\n' >&2
  exit 1
}
# shellcheck disable=SC2034 # read by scenario_uninstall_dev.sh's dev-install scenario
REAL_CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
# shellcheck disable=SC2034 # read by scenario_uninstall_dev.sh's dev-install scenario
REAL_RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"

SERVED_ROOT="$(mktemp -d /tmp/cdi-srv.XXXXXX)"
BUILD_DIR="${DAEMON_INSTALL_BUILD_DIR:-$PROJECT_ROOT/target/lifecycle-build}"
SERVER_PID=""
cleanup() {
  [ -z "$SERVER_PID" ] || kill "$SERVER_PID" 2>/dev/null || true
  rm -rf "$SERVED_ROOT"
}
trap cleanup EXIT

if [ -n "$NEW_BIN" ]; then
  LIFECYCLE1_BIN="$NEW_BIN"
  LIFECYCLE2_BIN="$NEXT_BIN"
  LIFECYCLE1_TAG="$(lifecycle_tag 1)"
  LIFECYCLE2_TAG="$(lifecycle_tag 2)"
else
  IFS=$'\t' read -r LIFECYCLE1_BIN LIFECYCLE1_TAG < <(build_lifecycle 1 "$BUILD_DIR")
  IFS=$'\t' read -r LIFECYCLE2_BIN LIFECYCLE2_TAG < <(build_lifecycle 2 "$BUILD_DIR")
fi
package_lifecycle_tag "$SERVED_ROOT" "$TARGET" "$LIFECYCLE1_BIN" "$LIFECYCLE1_TAG"
package_lifecycle_tag "$SERVED_ROOT" "$TARGET" "$LIFECYCLE2_BIN" "$LIFECYCLE2_TAG"
stage_old_releases "$SERVED_ROOT" "$TARGET"

"${PYTHON:-python3}" "$LIB/release_server.py" "$SERVED_ROOT" "$LIFECYCLE1_TAG" >"$SERVED_ROOT/.port" &
SERVER_PID=$!
for _ in $(seq 1 100); do
  [ -s "$SERVED_ROOT/.port" ] && break
  sleep 0.1
done
SERVER_BASE="http://127.0.0.1:$(head -1 "$SERVED_ROOT/.port")"

if [ -n "$SCENARIO" ]; then
  run_scenario "$SCENARIO"
else
  for name in "${SCENARIO_NAMES[@]}"; do
    run_scenario "$name"
  done
fi

[ "$DAEMON_INSTALL_FAILED" -eq 0 ]
