#!/usr/bin/env bash
# Apply rustfmt formatting in place. Sibling of fmt-check.sh: same nightly-key
# warning filter (imports_granularity, group_imports print a "Warning: can't
# set ..." line on stable), but writes the files instead of just checking.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

cd "$PROJECT_ROOT"
cargo fmt 2> >(grep -v "Warning: can't set" >&2 || true)
