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
#   typos-check      typos.toml
#   cli-docs-check   docs/cli-reference.md vs the real --help output
#
# Retired in the toolu migration (folded into guardrails-check + lint-check):
#   test-placement-check  no-bypass-check  module-size-check  tests-mirror-check
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

GATES=(
  fmt-check
  type-check
  lint-check
  guardrails-check
  typos-check
  cli-docs-check
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
