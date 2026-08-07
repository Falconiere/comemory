#!/usr/bin/env bash
# Structure gate: the toolu-conventions agent-guardrails module.
#
# scripts/guardrails/ is copied VERBATIM from the kit and must never be
# hand-edited — change guardrails.config.json instead. The only local additions
# are the two project-local rules in patterns/rust/ (no-unsafe-without-safety,
# no-allow-attribute); see CLAUDE.md deviations D1/D6.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

cd "$PROJECT_ROOT"
require_cmd jq "apt-get install jq | brew install jq"
require_cmd ast-grep "cargo install ast-grep --locked"

# run.sh exits 3 on misconfiguration, 1 on violations, 0 clean.
bash scripts/guardrails/run.sh
