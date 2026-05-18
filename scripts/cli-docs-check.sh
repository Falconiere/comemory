#!/usr/bin/env bash
# Verify docs/cli-reference.md matches the output of scripts/regen-cli-docs.sh.

set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

STEP="cli-docs-check"
TMP="$(mktemp -t qwick-cli-reference.XXXXXX)"
trap 'rm -f "$TMP"' EXIT

log_info "$STEP" "regenerating into $TMP"
bash "$HERE/regen-cli-docs.sh" "$TMP" >/dev/null

if ! diff -u "$PROJECT_ROOT/docs/cli-reference.md" "$TMP"; then
  printf "%s[%s]%s docs/cli-reference.md is stale; run %sbash scripts/regen-cli-docs.sh%s\n" \
    "$C_RED" "$STEP" "$C_RST" "$C_YLW" "$C_RST" 1>&2
  exit 1
fi

log_ok "$STEP" "docs/cli-reference.md is up to date"
