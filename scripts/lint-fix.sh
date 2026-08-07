#!/usr/bin/env bash
# Apply clippy's machine-fixable suggestions in place. Sibling of
# lint-check.sh: same command plus `--fix --allow-dirty --allow-staged` so it
# writes files instead of just checking. Does not replace hand-fixing —
# clippy's `--fix` only rewrites lints it can prove are behavior-preserving.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"
run_cargo clippy --all-targets --all-features --fix --allow-dirty --allow-staged -- -D warnings
