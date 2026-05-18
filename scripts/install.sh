#!/usr/bin/env bash
# Build a release binary of qwick-memory and install it into the user's
# cargo bin directory (typically ~/.cargo/bin). Run from anywhere.

set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

STEP="install"

WITH_TOOLS=0
[[ "${1:-}" == "--with-tools" ]] && WITH_TOOLS=1

log_info "$STEP" "building release-quick binary"
run_cargo build --profile release-quick --locked

log_info "$STEP" "installing into cargo bin (release-quick profile)"
run_cargo install --path "$PROJECT_ROOT" --profile release-quick --locked --force

if [[ "$WITH_TOOLS" -eq 1 ]]; then
  if command -v brew >/dev/null 2>&1; then
    log_info "$STEP" "installing optional tools (sccache, hyperfine)"
    brew install sccache hyperfine
  else
    log_info "$STEP" "brew not found; skipping optional tools (--with-tools)"
  fi
fi

BIN_DIR="${CARGO_INSTALL_ROOT:-${CARGO_HOME:-$HOME/.cargo}/bin}"
BIN_PATH="$BIN_DIR/qwick-memory"

if [[ ! -x "$BIN_PATH" ]]; then
  die "$STEP" "expected binary at $BIN_PATH but none found"
fi

INSTALLED_VERSION="$("$BIN_PATH" --version 2>/dev/null || true)"
log_ok "$STEP" "installed $BIN_PATH ($INSTALLED_VERSION)"

case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) printf "%s[%s]%s note: %s is not on PATH\n" "$C_YLW" "$STEP" "$C_RST" "$BIN_DIR" ;;
esac
