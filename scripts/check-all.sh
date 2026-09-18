#!/usr/bin/env bash
# Run every quality gate. Exit 1 on first failure.
#
# Gate ownership (one rule, one enforcer):
#   fmt-check        rustfmt.toml
#   type-check       cargo check
#   lint-check       Cargo.toml [lints] + clippy.toml  (unwrap/expect/panic/
#                    todo/unimplemented/print_*/too_many_lines/pedantic)
#   guardrails-check guardrails.config.json + scripts/guardrails/  (file size,
#                    folder tree, no mod.rs barrels, snake_case filenames,
#                    secrets, shadow configs, ast-grep patterns: no inline test
#                    module, no direct env read, no unsafe without SAFETY,
#                    no #[allow])
#   architecture-check  scripts/architecture-check.sh  (staged domain ownership
#                    and temporary dependency policy)
#   store-chokepoint-check  store-leak-baseline.txt  (two-sided ratchet: no
#                    production file outside src/store/ except src/errors.rs
#                    may import rusqlite; no store/ function returns a driver
#                    cursor type)
#   typos-check      typos.toml
#   cli-docs-check   docs/cli-reference.md vs the real --help output
#   migration-check  every already-released migrations/*.sql file is
#                    byte-identical to its content at the first release tag
#                    that shipped it (git-dependent, requires unshallow tags)
#
# Retired in the toolu migration (folded into guardrails-check + lint-check):
#   test-placement-check  no-bypass-check  module-size-check  tests-mirror-check
#
# DELIBERATELY NOT HERE: dup-check, deny-check, machete-check. Each needs a
# separately installed tool (similarity-rs, cargo-deny, cargo-machete), and this
# script runs on every local iteration and on the release path — so adding one
# would mean either hard-failing every contributor who lacks the tool or
# skipping silently, and a guardrail that reports success while checking nothing
# is exactly what these gates exist to prevent. They are grouped in the
# justfile's `qa` recipe instead, and ALL THREE now also run as their own steps
# in .github/workflows/test.yml, where their pinned tools are guaranteed —
# SIMILARITY_RS_VERSION, CARGO_DENY_VERSION and CARGO_MACHETE_VERSION, each a
# single constant in its own script that the workflow derives from rather than
# repeats. Omitting them here therefore costs no enforcement at all.
# Do not "fix" this omission by adding them to GATES below.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

GATES=(
  fmt-check
  type-check
  lint-check
  guardrails-check
  architecture-check
  store-chokepoint-check
  typos-check
  cli-docs-check
  migration-check
)

failed=()
for g in "${GATES[@]}"; do
  log_info "$g" "running"
  if bash "$HERE/$g.sh"; then
    log_ok "$g"
  else
    log_err "$g" "failed"
    failed+=("$g")
  fi
done

if (( ${#failed[@]} > 0 )); then
  log_err "check-all" "${#failed[@]} gate(s) failed: ${failed[*]}"
  exit 1
fi
log_ok "check-all" "all gates passed"
