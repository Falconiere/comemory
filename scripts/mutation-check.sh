#!/usr/bin/env bash
# Run cargo-mutants. A surviving mutant (a change no test caught) makes the
# command exit non-zero; callers that want report-only behaviour (the PR job)
# set continue-on-error at the workflow level, not here.
#
# Usage:
#   mutation-check.sh diff <diff-file>   # mutate only lines in the diff (PR)
#   mutation-check.sh full               # mutate the whole crate (nightly)
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"
cd "$PROJECT_ROOT"

require_cmd cargo-mutants "cargo install cargo-mutants"

mode="${1:-}"
case "$mode" in
  diff)
    diff_file="${2:-}"
    [[ -n "$diff_file" && -f "$diff_file" ]] \
      || die "mutation-check" "usage: mutation-check.sh diff <diff-file>"
    log_info "mutation-check" "diff-scoped mutants over $diff_file"
    run_cargo mutants --all-features --in-diff "$diff_file"
    ;;
  full)
    log_info "mutation-check" "full-crate mutants"
    run_cargo mutants --all-features
    ;;
  *)
    die "mutation-check" "usage: mutation-check.sh diff <diff-file> | full"
    ;;
esac
log_ok "mutation-check" "no surviving mutants"
