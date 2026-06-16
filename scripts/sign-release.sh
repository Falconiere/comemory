#!/usr/bin/env bash
# scripts/sign-release.sh — opt-in minisign signature for the SHA256SUMS
# attached to a GitHub release. Used by .github/workflows/release-finalize.yml.
#
# Behaviour:
#   - If `minisign` is on PATH: download SHA256SUMS, sign it with the
#     secret key (path from $MINISIGN_KEY env or the conventional
#     $HOME/.minisign/comemory.key location), and upload the .sig back
#     to the release with `gh release upload`.
#   - If `minisign` is absent OR the key is missing: print a yellow
#     warning and exit 0. The release is still published, just without
#     a detached signature.
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

key_path="${MINISIGN_KEY:-$HOME/.minisign/comemory.key}"
if [[ ! -f "$key_path" ]]; then
  log_info "$step" "no key at $key_path — skipping signature"
  log_ok "$step" "skipped (no key)"
  exit 0
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

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

# Sign. Use the passphrase non-interactively if MINISIGN_PASSPHRASE is set;
# otherwise fall back to minisign's own TTY prompt (caller decides).
sign_args=("-s" "$key_path" "-m" "$work/SHA256SUMS" "-W")
if [[ -n "${MINISIGN_PASSPHRASE:-}" ]]; then
  sign_args=("-s" "$key_path" "-m" "$work/SHA256SUMS" \
    "-W" "-P" "$MINISIGN_PASSPHRASE")
fi

if ! minisign "${sign_args[@]}" >/dev/null 2>&1; then
  log_err "$step" "minisign failed (bad passphrase or corrupt key)"
  exit 1
fi

# Upload the signature back to the release.
gh release upload "$ver" "$work/SHA256SUMS.minisig" --clobber

log_ok "$step" "uploaded SHA256SUMS.minisig to release $ver"
