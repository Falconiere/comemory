#!/usr/bin/env bash
# Build a release binary of qwick-memory and install it into the user's
# cargo bin directory (typically ~/.cargo/bin). Run from anywhere.

set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

STEP="install"

WITH_TOOLS=0
CLEAN=1
INSTALL_COMPLETIONS=1
for arg in "$@"; do
  case "$arg" in
    --with-tools)     WITH_TOOLS=1 ;;
    --no-clean)       CLEAN=0 ;;
    --no-completions) INSTALL_COMPLETIONS=0 ;;
    *) die "$STEP" "unknown argument: $arg (expected --with-tools, --no-clean, or --no-completions)" ;;
  esac
done

if [[ "$CLEAN" -eq 1 ]]; then
  if command -v uv >/dev/null 2>&1 \
     && uv tool list 2>/dev/null | awk '{print $1}' | grep -qx 'qwick-memory'; then
    log_info "$STEP" "detected uv tool: qwick-memory — uninstalling"
    uv tool uninstall qwick-memory >/dev/null
  fi
  if command -v brew >/dev/null 2>&1 \
     && brew list --formula 2>/dev/null | grep -qx 'qwick-memory'; then
    log_info "$STEP" "detected brew formula: qwick-memory — uninstalling"
    brew uninstall qwick-memory >/dev/null
  fi
fi

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

SHADOW=""
IFS=':' read -r -a PATH_PARTS <<< "$PATH"
for p in "${PATH_PARTS[@]}"; do
  candidate="$p/qwick-memory"
  if [[ -x "$candidate" && "$candidate" != "$BIN_PATH" ]]; then
    SHADOW="$candidate"
    break
  fi
done
if [[ -n "$SHADOW" ]]; then
  printf "%s[%s]%s warning: %s appears on PATH before %s — rehash your shell or remove the shadow\n" \
    "$C_YLW" "$STEP" "$C_RST" "$SHADOW" "$BIN_PATH"
fi

# Install shell completions for any detected shell. Writes to canonical
# autoload paths so no manual sourcing is required after a new shell session.
install_completion() {
  local shell="$1"
  local dest="$2"
  local dir
  dir="$(dirname "$dest")"
  if ! mkdir -p "$dir" 2>/dev/null; then
    log_info "$STEP" "completion[$shell]: cannot create $dir; skipping"
    return
  fi
  if ! "$BIN_PATH" completions "$shell" > "$dest.tmp" 2>/dev/null; then
    rm -f "$dest.tmp"
    log_info "$STEP" "completion[$shell]: generation failed; skipping"
    return
  fi
  mv "$dest.tmp" "$dest"
  log_ok "$STEP" "completion[$shell] -> $dest"
}

if [[ "$INSTALL_COMPLETIONS" -eq 1 ]]; then
  if command -v fish >/dev/null 2>&1; then
    install_completion fish "${XDG_CONFIG_HOME:-$HOME/.config}/fish/completions/qwick-memory.fish"
  fi

  if command -v zsh >/dev/null 2>&1; then
    ZSH_DEST=""
    if command -v brew >/dev/null 2>&1; then
      ZSH_BREW_DIR="$(brew --prefix 2>/dev/null)/share/zsh/site-functions"
      [[ -d "$ZSH_BREW_DIR" && -w "$ZSH_BREW_DIR" ]] && ZSH_DEST="$ZSH_BREW_DIR/_qwick-memory"
    fi
    if [[ -z "$ZSH_DEST" ]]; then
      ZSH_DEST="$HOME/.zfunc/_qwick-memory"
      log_info "$STEP" "zsh: installing to ~/.zfunc; add 'fpath=(~/.zfunc \$fpath)' before 'compinit' in ~/.zshrc if not present"
    fi
    install_completion zsh "$ZSH_DEST"
  fi

  if command -v bash >/dev/null 2>&1; then
    BASH_DEST=""
    if command -v brew >/dev/null 2>&1; then
      BASH_BREW_DIR="$(brew --prefix 2>/dev/null)/etc/bash_completion.d"
      [[ -d "$BASH_BREW_DIR" && -w "$BASH_BREW_DIR" ]] && BASH_DEST="$BASH_BREW_DIR/qwick-memory"
    fi
    if [[ -z "$BASH_DEST" ]]; then
      BASH_DEST="$HOME/.local/share/bash-completion/completions/qwick-memory"
    fi
    install_completion bash "$BASH_DEST"
  fi
fi
