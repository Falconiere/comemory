#!/usr/bin/env bash
# scripts/sign-release.sh — opt-in minisign signature for the SHA256SUMS
# attached to a GitHub release. Used by .github/workflows/release-finalize.yml.
#
# Behaviour:
#   - If `minisign` is on PATH: download SHA256SUMS, sign it with the
#     secret key, and upload the .sig back to the release with
#     `gh release upload`.
#   - If `minisign` is absent OR the key is missing: print a yellow
#     warning and exit 0. The release is still published, just without
#     a detached signature.
#
# Key resolution:
#   - $MINISIGN_KEY env var set (with the key contents, as set by
#     `gh secret set MINISIGN_KEY < key.file` per keys/README.md):
#     write the contents to a 0600 temp file and use that as the key
#     path. Cleaned up on exit.
#   - $MINISIGN_KEY unset: fall back to $HOME/.minisign/comemory.key
#     (override with $MINISIGN_KEY_PATH if you need a different path).
#
# Passphrase:
#   - $MINISIGN_PASSPHRASE set: pipe via stdin so the secret never
#     appears in argv (visible in `ps` / /proc/*/cmdline for the
#     duration of the call). minisign reads the passphrase from stdin
#     when stdin is not a TTY — the default on a CI runner.
#
# This script is opt-in: missing tools never fail the release workflow.
# Configure it by setting the MINISIGN_KEY and MINISIGN_PASSPHRASE GitHub
# Actions secrets, and committing comemory.pub to keys/.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

step="sign-release"
ver="${1:-}"
if [[ -z "$ver" ]]; then
  die "$step" "usage: bash $0 <version-tag>  (e.g. v0.10.0)"
fi

# Soft skip: no minisign installed.
if ! command -v minisign >/dev/null 2>&1; then
  log_info "$step" "minisign not installed — skipping SHA256SUMS signature"
  log_ok "$step" "skipped (no minisign)"
  exit 0
fi

# Soft skip: no gh available (this script is meant to be called from
# release-finalize.yml, where gh is preinstalled on the runner; if it
# isn't, we still don't want to fail the release).
if ! command -v gh >/dev/null 2>&1; then
  log_info "$step" "gh not installed — skipping SHA256SUMS signature"
  log_ok "$step" "skipped (no gh)"
  exit 0
fi

# Resolve the secret key path. Two modes (see header):
#   1. MINISIGN_KEY holds the key contents → write to a 0600 temp file
#      (tracked in $key_cleanup so the EXIT trap can remove it).
#   2. MINISIGN_KEY unset → fall back to $MINISIGN_KEY_PATH or
#      $HOME/.minisign/comemory.key.
key_cleanup=""
if [[ -n "${MINISIGN_KEY:-}" ]]; then
  key_path="$(mktemp)"
  chmod 600 "$key_path"
  printf '%s' "$MINISIGN_KEY" > "$key_path"
  key_cleanup="$key_path"
else
  key_path="${MINISIGN_KEY_PATH:-$HOME/.minisign/comemory.key}"
fi

if [[ ! -s "$key_path" ]]; then
  log_info "$step" "no key at $key_path — skipping signature"
  log_ok "$step" "skipped (no key)"
  [[ -n "$key_cleanup" ]] && rm -f "$key_cleanup"
  exit 0
fi

work="$(mktemp -d)"
cleanup() {
  rm -rf "$work"
  [[ -n "$key_cleanup" ]] && rm -f "$key_cleanup"
}
trap cleanup EXIT

# Download SHA256SUMS from the release.
if ! gh release download "$ver" --pattern SHA256SUMS --dir "$work" >/dev/null 2>&1; then
  log_info "$step" "no SHA256SUMS artifact on release $ver — skipping"
  log_ok "$step" "skipped (no SHA256SUMS)"
  exit 0
fi

if [[ ! -s "$work/SHA256SUMS" ]]; then
  log_info "$step" "empty SHA256SUMS — skipping"
  log_ok "$step" "skipped (empty SHA256SUMS)"
  exit 0
fi

# Sign. Pass the passphrase via stdin (not -P) so the secret never
# appears in argv, which is visible in `ps` / /proc/*/cmdline for the
# duration of the call. minisign reads the passphrase from stdin when
# stdin is not a TTY — true on a CI runner. The trailing newline is
# required: minisign uses fgets() and trims it.
if [[ -n "${MINISIGN_PASSPHRASE:-}" ]]; then
  if ! printf '%s\n' "$MINISIGN_PASSPHRASE" \
      | minisign -W -s "$key_path" -m "$work/SHA256SUMS" >/dev/null 2>&1; then
    log_err "$step" "minisign failed (bad passphrase, corrupt key, or wrong version)"
    exit 1
  fi
else
  if ! minisign -W -s "$key_path" -m "$work/SHA256SUMS" >/dev/null 2>&1; then
    log_err "$step" "minisign failed (corrupt key or wrong version)"
    exit 1
  fi
fi

# Upload the signature back to the release.
gh release upload "$ver" "$work/SHA256SUMS.minisig" --clobber

log_ok "$step" "uploaded SHA256SUMS.minisig to release $ver"
