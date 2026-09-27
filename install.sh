#!/bin/sh
# install.sh — the comemory installer.
#
# Downloads the release archive built for this machine from GitHub Releases,
# verifies it against the SHA-256 sidecar every release ships, checks the
# binary actually runs here, drops it into a bin directory, starts its
# required sync daemon and proves the new binary answers, and (unless told
# not to) makes it reachable from your shell. Exit 69: the binary was placed
# but its daemon never became ready (the previous binary is restored).
#
#   curl -fsSL https://github.com/Falconiere/comemory/releases/latest/download/install.sh | sh
#   sh install.sh [--version <tag>] [--dir <bin dir>] [--no-modify-path]
#                 [--no-completions] [--quiet]
#
# Environment (a flag wins over its variable):
#   COMEMORY_VERSION         release to install (v0.18.2 or 0.18.2); default latest
#   COMEMORY_INSTALL_DIR     directory the binary goes in; default: see pick_dir
#   COMEMORY_NO_MODIFY_PATH  non-empty → leave shell rc files alone
#   COMEMORY_RELEASES_URL    release base (test hook); default
#                            https://github.com/Falconiere/comemory/releases
#   NO_COLOR                 non-empty → plain output
#
# POSIX sh — runs under dash, ash, bash, and zsh. `comemory upgrade` runs this
# same script with --version / --dir / --no-modify-path / --quiet, so those
# four flags are a contract: the binary depends on them. --no-completions is
# the user-facing opt-out for the installer's default completion setup.
set -eu

APP=comemory
REPO_URL=https://github.com/Falconiere/comemory
RELEASES="${COMEMORY_RELEASES_URL:-$REPO_URL/releases}"
VERSION="${COMEMORY_VERSION:-}"
DIR="${COMEMORY_INSTALL_DIR:-}"
MODIFY_PATH=1
[ -z "${COMEMORY_NO_MODIFY_PATH:-}" ] || MODIFY_PATH=0
QUIET=0
INSTALL_COMPLETIONS=1
NEEDS_RESTART=0
TMP=""
LOCK=""
PREV=""

# ---------------------------------------------------------------- ui ----
if [ -t 1 ] && [ -z "${NO_COLOR:-}" ] && [ "${TERM:-dumb}" != dumb ]; then
  B="$(printf '\033[1m')"; D="$(printf '\033[2m')"; R="$(printf '\033[0m')"
  GRN="$(printf '\033[32m')"; RED="$(printf '\033[31m')"
  YLW="$(printf '\033[33m')"; CYN="$(printf '\033[36m')"
else
  B=""; D=""; R=""; GRN=""; RED=""; YLW=""; CYN=""
fi
case "${LC_ALL:-${LC_CTYPE:-${LANG:-}}}" in
  *[Uu][Tt][Ff]-8*|*[Uu][Tt][Ff]8*) OK="✓"; BAD="✗"; ARROW="→"; DOT="·" ;;
  *) OK="ok"; BAD="!!"; ARROW="->"; DOT="-" ;;
esac

say()  { [ "$QUIET" -eq 1 ] || printf '%s\n' "$*"; }
step() { say "  ${GRN}${OK}${R} ${B}$1${R}  ${D}$2${R}"; }
info() { say "  ${CYN}${ARROW}${R} ${B}$1${R}  ${D}$2${R}"; }
warn() { printf '  %s%s%s %s\n' "$YLW" "$BAD" "$R" "$*" >&2; }
# fail <code> <message> [hint] — red error line, optional indented hint.
fail() {
  code="$1"; shift
  printf '\n  %s%s error:%s %s\n' "$RED" "$BAD" "$R" "$1" >&2
  [ $# -lt 2 ] || printf '    %s%s%s\n' "$D" "$2" "$R" >&2
  exit "$code"
}
die() { fail 1 "$@"; }
have() { command -v "$1" >/dev/null 2>&1; }

usage() {
  cat <<EOF
comemory installer

usage: install.sh [options]

  --version <tag>     release to install (v0.18.2 or 0.18.2); default: latest
  --dir <path>        directory the binary goes in (default: the existing
                      comemory's directory, else \$CARGO_HOME/bin if it exists,
                      else ~/.local/bin)
  --no-modify-path    do not add the install directory to shell rc files
  --no-completions    do not install shell completion scripts
  --quiet, -q         only print errors
  --help, -h          this text

env: COMEMORY_VERSION, COMEMORY_INSTALL_DIR, COMEMORY_NO_MODIFY_PATH, NO_COLOR
EOF
}

banner() {
  say ""
  say "  ${B}${CYN}comemory${R} ${D}${DOT} installer${R}"
  say "  ${D}agentic dev memory + code-aware semantic search${R}"
  say ""
}

# ---------------------------------------------------------- platform ----
detect_target() {
  os="$(uname -s)"; arch="$(uname -m)"
  case "$os:$arch" in
    Darwin:arm64)              TARGET=aarch64-apple-darwin;      PRETTY="macOS Apple Silicon" ;;
    Linux:x86_64)              TARGET=x86_64-unknown-linux-gnu;  PRETTY="Linux x86_64" ;;
    Linux:aarch64|Linux:arm64) TARGET=aarch64-unknown-linux-gnu; PRETTY="Linux aarch64" ;;
    *) die "no prebuilt $APP for $os $arch" \
         "build from source: git clone $REPO_URL && cd $APP && bash scripts/dev-install.sh" ;;
  esac
  if [ "$os" = Linux ] && ls /lib/ld-musl-* >/dev/null 2>&1; then
    die "this is a musl system; only glibc (>= 2.35) Linux builds are published" \
      "build from source: git clone $REPO_URL && cd $APP && bash scripts/dev-install.sh"
  fi
}

# ---------------------------------------------------------- download ----
# fetch <url> <dest> [progress] — curl, else wget. TLS hardening only applies
# to https URLs so a loopback COMEMORY_RELEASES_URL test server still works.
fetch() {
  proto=""
  case "$1" in https://*) proto="--proto =https --proto-redir =https --tlsv1.2" ;; esac
  if have curl; then
    if [ "${3:-}" = progress ] && [ -t 2 ] && [ "$QUIET" -eq 0 ]; then
      # shellcheck disable=SC2086
      curl $proto -fL --progress-bar -o "$2" "$1"
    else
      # shellcheck disable=SC2086
      curl $proto -fsSL -o "$2" "$1"
    fi
  elif have wget; then
    wget -q -O "$2" "$1"
  else
    die "need curl or wget on PATH to download $APP"
  fi
}

# Resolve `latest` by following the redirect GitHub serves for
# <releases>/latest; the final URL ends in the tag.
resolve_version() {
  if [ -n "$VERSION" ] && [ "$VERSION" != latest ]; then
    case "$VERSION" in v*) ;; *) VERSION="v$VERSION" ;; esac
    step "Version" "$VERSION (pinned)"
    return
  fi
  url="$RELEASES/latest"; final=""
  if have curl; then
    final="$(curl -fsSLI -o /dev/null -w '%{url_effective}' "$url" 2>/dev/null)" || final=""
  elif have wget; then
    final="$(wget -q -S --spider "$url" 2>&1 | grep -i '^ *location:' | tail -n 1 \
      | sed 's/^ *[^:]*: *//; s/ .*//')" || final=""
  fi
  VERSION="${final##*/}"
  case "$VERSION" in
    v[0-9]*) ;;
    *) die "could not resolve the latest release from $url" \
         "pin one with --version vX.Y.Z — see $REPO_URL/releases" ;;
  esac
  step "Version" "$VERSION (latest)"
}

# ------------------------------------------------------------ verify ----
verify_sha256() {
  if have sha256sum; then got="$(sha256sum "$1" | cut -d' ' -f1)"
  elif have shasum; then got="$(shasum -a 256 "$1" | cut -d' ' -f1)"
  elif have openssl; then got="$(openssl dgst -sha256 "$1" | sed 's/.* //')"
  else die "no sha256sum, shasum, or openssl on PATH to verify the download"
  fi
  [ "$got" = "$2" ] || die "checksum mismatch for $ARCHIVE" \
    "expected $2, got $got — corrupt or tampered download; nothing was installed"
}

# --------------------------------------------------------- install dir ----
# Precedence: --dir / COMEMORY_INSTALL_DIR → the directory of the comemory
# already on PATH (so a re-run upgrades in place) → $CARGO_HOME/bin when it
# exists (where the cargo-dist installer used to put it) → ~/.local/bin.
pick_dir() {
  if [ -n "$DIR" ]; then WHY="--dir"; return; fi
  existing="$(command -v "$APP" 2>/dev/null || true)"
  if [ -n "$existing" ]; then
    link="$(readlink "$existing" 2>/dev/null || true)"
    case "$existing:$link" in
      *Cellar*|*/opt/homebrew/*|*/linuxbrew/*)
        warn "a Homebrew $APP lives at $existing — upgrade that one with: brew upgrade $APP" ;;
      *) DIR="$(cd "$(dirname "$existing")" && pwd)"; WHY="replacing $existing"; return ;;
    esac
  fi
  cargo_bin="${CARGO_HOME:-$HOME/.cargo}/bin"
  if [ -d "$cargo_bin" ]; then DIR="$cargo_bin"; WHY="cargo bin directory"
  else DIR="$HOME/.local/bin"; WHY="default"
  fi
}

# ------------------------------------------------------------- lock ----
# One install per directory: a symlink naming the owner pid (`ln -s` is
# atomic and carries the pid). A dead owner's lock is reclaimed only by the
# one racer that creates `.reclaim.<pid>`; a reclaimer that died mid-way
# leaves a state no installer guesses about.
alive() { ps -p "$1" >/dev/null 2>&1; }
lock_dir() {
  LOCK="$DIR/.$APP-install.lock"; waited=0
  until ln -s "$$" "$LOCK" 2>/dev/null; do
    owner="$(readlink "$LOCK" 2>/dev/null || true)"
    if [ -n "$owner" ] && ! alive "$owner"; then
      if ln -s "$$" "$LOCK.reclaim.$owner" 2>/dev/null; then
        ln -sf "$$" "$LOCK.new.$$" && mv -f "$LOCK.new.$$" "$LOCK" && break
      fi
      by="$(readlink "$LOCK.reclaim.$owner" 2>/dev/null || true)"
      [ -z "$by" ] || alive "$by" || die "a previous install into $DIR was interrupted while taking its lock" \
        "remove $DIR/.$APP-install.lock* and retry"
    fi
    [ "$waited" -lt 60 ] || die "another install into $DIR is running (pid ${owner:-?})" "retry when it finishes"
    waited=$((waited + 1)); sleep 1
  done
  for f in "$DIR/.$APP.new."* "$DIR/.$APP.prev."*; do
    pid="${f##*.}"
    case "$pid" in ''|*[!0-9]*) continue ;; esac
    alive "$pid" || rm -f "$f"
  done
}
cleanup() {
  rm -rf "$TMP"; rm -f "$DIR/.$APP.new.$$"
  [ -z "$LOCK" ] || [ "$(readlink "$LOCK" 2>/dev/null)" != "$$" ] || rm -f "$LOCK" "$LOCK".reclaim.*
}

# Extract, prove the binary runs on this machine, then rename it into place.
# The final step is a same-directory rename, so a running comemory (this is
# what `comemory upgrade` calls) is swapped atomically rather than written
# over. The previous file is kept as a hard link until the daemon proves the
# new one.
install_bin() {
  tar -xJf "$TMP/$ARCHIVE" -C "$TMP" 2>/dev/null \
    || die "could not extract $ARCHIVE" "tar with xz support is required (tar -xJf)"
  bin="$TMP/$APP-$TARGET/$APP"
  [ -f "$bin" ] || die "$ARCHIVE does not contain $APP-$TARGET/$APP"
  chmod 755 "$bin"
  REPORTED="$("$bin" --version 2>&1)" || die "the downloaded binary does not run here: $REPORTED" \
    "Linux builds need glibc >= 2.35; older distros need the source install"
  mkdir -p "$DIR" 2>/dev/null || die "cannot create $DIR" "re-run with --dir <a directory you can write to>"
  [ -w "$DIR" ] || die "cannot write to $DIR" "re-run with --dir <a directory you can write to>"
  lock_dir
  staged="$DIR/.$APP.new.$$"
  mv -f "$bin" "$staged" || die "cannot stage the binary in $DIR"
  if [ -f "$DIR/$APP" ]; then
    PREV="$DIR/.$APP.prev.$$"
    ln "$DIR/$APP" "$PREV" 2>/dev/null || cp -p "$DIR/$APP" "$PREV" \
      || die "cannot keep the previous $APP for rollback"
    PREV_REPORTED="$("$DIR/$APP" --version 2>/dev/null || echo "the previous $APP")"
  fi
  mv -f "$staged" "$DIR/$APP"
}

# ----------------------------------------------------------- daemon ----
# json_str <json> <key> — a `"key":"value"` string out of compact JSON.
json_str() { printf '%s' "$1" | sed -n "s/.*\"$2\":[[:space:]]*\"\([^\"]*\)\".*/\\1/p"; }

# The new binary's own `sync daemon ensure` must answer with this version at
# this path. Otherwise the previous binary comes back and the install fails.
ensure_daemon() {
  want="${REPORTED#* }"; path="$(cd -P "$DIR" && pwd -P)/$APP"
  out="$("$DIR/$APP" sync daemon ensure --json 2>"$TMP/ensure.err")" || true
  got_v="$(json_str "$out" version)"; got_b="$(json_str "$out" binary)"
  case "$out" in *'"ready":true'*)
    if [ "$got_v" = "$want" ] && [ "$got_b" = "$path" ]; then
      rm -f "$PREV"; PREV=""
      pid="$(printf '%s' "$out" | sed -n 's/.*"pid":\([0-9]*\).*/\1/p')"
      step "Sync daemon" "ready ($got_v, pid $pid, $(json_str "$out" supervisor)) $ARROW $(json_str "$out" data_dir)"
      return 0
    fi ;;
  esac
  cause="$(json_str "$out" error)"
  [ -n "$cause" ] || cause="$(tail -n 1 "$TMP/ensure.err" 2>/dev/null)"
  [ -n "$cause" ] || cause="it answered as ${got_v:-nothing} at ${got_b:-nowhere}, expected $want at $path"
  what="sync daemon not ready after installing $APP $VERSION: $cause"
  [ -n "$PREV" ] || fail 69 "$what" "$APP $VERSION is installed at $DIR/$APP; fix the cause, then run: $DIR/$APP sync daemon ensure"
  mv -f "$PREV" "$DIR/$APP"; PREV=""
  back="$("$DIR/$APP" sync daemon ensure --json 2>/dev/null)" || true
  case "$back" in *'"ready":true'*) state=ready ;; *) state="not ready" ;; esac
  case "$back" in *'"store":"too_new"'*)
    state="$state; the store was written by a newer $APP: keep the newer one and run $APP doctor" ;;
  esac
  fail 69 "$what" "rolled back to $PREV_REPORTED at $DIR/$APP (sync daemon: $state)"
}

# Generate all supported completion scripts with the installed binary. A
# completion failure does not roll back a verified binary; report it so the
# user can retry with `comemory completions --install`.
install_completions() {
  [ "$INSTALL_COMPLETIONS" -eq 1 ] || return 0
  if "$DIR/$APP" completions --install >/dev/null; then
    step "Completions" "bash, zsh, fish, powershell"
  elif [ "$QUIET" -eq 0 ]; then
    warn "binary installed, but shell completion setup failed; retry: $APP completions --install"
  fi
}

# ------------------------------------------------------------- PATH ----
on_path() { case ":$PATH:" in *":$1:"*) return 0 ;; esac; return 1; }

add_path() {
  on_path "$DIR" && return 0
  NEEDS_RESTART=1
  if [ "$MODIFY_PATH" -eq 0 ]; then
    info "PATH" "$DIR is not on PATH; PATH rc changes skipped (--no-modify-path)"
    return 0
  fi
  case "$DIR" in "$HOME"/*) short="\$HOME${DIR#"$HOME"}" ;; *) short="$DIR" ;; esac
  case "$(basename "${SHELL:-sh}")" in
    fish) rc="${XDG_CONFIG_HOME:-$HOME/.config}/fish/conf.d/$APP.fish"
          line="fish_add_path --global \"$short\"" ;;
    zsh)  rc="${ZDOTDIR:-$HOME}/.zshrc";  line="export PATH=\"$short:\$PATH\"" ;;
    bash) rc="$HOME/.bashrc"; [ "$(uname -s)" != Darwin ] || rc="$HOME/.bash_profile"
          line="export PATH=\"$short:\$PATH\"" ;;
    *)    rc="$HOME/.profile"; line="export PATH=\"$short:\$PATH\"" ;;
  esac
  mkdir -p "$(dirname "$rc")"
  if [ -f "$rc" ] && grep -Fqx "$line" "$rc" 2>/dev/null; then
    step "PATH" "$rc already adds $short"
  else
    printf '\n# added by the comemory installer\n%s\n' "$line" >> "$rc"
    step "PATH" "added $short to $rc"
  fi
}

# ---------------------------------------------------------- summary ----
summary() {
  say ""
  say "  ${B}${GRN}${OK} $APP $VERSION installed${R}  ${D}${ARROW} $DIR/$APP${R}"
  say ""
  say "  ${B}next${R}"
  [ "$NEEDS_RESTART" -eq 0 ] || \
    say "    ${D}open a new shell, or:${R} export PATH=\"$DIR:\$PATH\""
  say "    $APP doctor      ${D}check the install${R}"
  say "    $APP --help      ${D}every command${R}"
  say "    $APP upgrade     ${D}later: move to the next release${R}"
  say ""
  say "  ${D}docs ${DOT} $REPO_URL/blob/main/docs/getting-started.md${R}"
  say ""
}

# ------------------------------------------------------------- main ----
while [ $# -gt 0 ]; do
  case "$1" in
    --version)   [ $# -ge 2 ] || die "--version needs a value"; VERSION="$2"; shift 2 ;;
    --version=*) VERSION="${1#*=}"; shift ;;
    --dir)       [ $# -ge 2 ] || die "--dir needs a value"; DIR="$2"; shift 2 ;;
    --dir=*)     DIR="${1#*=}"; shift ;;
    --no-modify-path) MODIFY_PATH=0; shift ;;
    --no-completions) INSTALL_COMPLETIONS=0; shift ;;
    --quiet|-q)  QUIET=1; shift ;;
    --help|-h)   usage; exit 0 ;;
    *) usage >&2; printf '\nerror: unknown option: %s\n' "$1" >&2; exit 2 ;;
  esac
done

banner
detect_target
step "Platform" "$PRETTY ($TARGET)"
resolve_version
pick_dir
step "Install dir" "$DIR ($WHY)"

TMP="$(mktemp -d 2>/dev/null || mktemp -d -t "$APP")"
trap cleanup EXIT
trap 'exit 130' INT TERM HUP
ARCHIVE="$APP-$TARGET.tar.xz"
base="$RELEASES/download/$VERSION"

info "Downloading" "$ARCHIVE"
fetch "$base/$ARCHIVE" "$TMP/$ARCHIVE" progress \
  || die "download failed: $base/$ARCHIVE" \
     "is $VERSION a published release with a $TARGET build? see $REPO_URL/releases"
fetch "$base/$ARCHIVE.sha256" "$TMP/$ARCHIVE.sha256" \
  || die "download failed: $base/$ARCHIVE.sha256"
expected="$(cut -d' ' -f1 < "$TMP/$ARCHIVE.sha256")"
verify_sha256 "$TMP/$ARCHIVE" "$expected"
step "Checksum" "sha256 $(printf '%.12s' "$expected")... verified"

install_bin
step "Installed" "$DIR/$APP ($REPORTED)"
ensure_daemon
install_completions
add_path
summary
