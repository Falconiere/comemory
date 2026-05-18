#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

if ! command -v typos >/dev/null 2>&1; then
  die "typos-check" "typos not installed (cargo install typos-cli)"
fi
cd "$PROJECT_ROOT" && typos
