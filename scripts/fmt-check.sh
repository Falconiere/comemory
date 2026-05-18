#!/usr/bin/env bash
# rustfmt check. rustfmt.toml carries two nightly-only keys
# (imports_granularity, group_imports) that print a "Warning: can't set ..."
# line to stderr on stable. We grep those specific lines out so the gate is
# quiet on a green run; everything else still flows to stderr and we exit
# with cargo fmt's own status (preserved by `set -o pipefail`).
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

cd "$PROJECT_ROOT"
# Redirect stderr through a grep -v filter, then back to stderr.
# The `|| true` on grep is required because grep -v exits 1 when *all* lines
# match the inverted pattern (i.e. stderr was nothing but nightly warnings).
cargo fmt --check 2> >(grep -v "Warning: can't set" >&2 || true)
