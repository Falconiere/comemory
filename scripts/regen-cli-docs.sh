#!/usr/bin/env bash
# Regenerate docs/cli-reference.md from `qwick-memory <cmd> --help` output.
# This is the single source of truth for the CLI reference page.

set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

STEP="regen-cli-docs"

OUT="${1:-$PROJECT_ROOT/docs/cli-reference.md}"

log_info "$STEP" "building release-quick binary"
run_cargo build --profile release-quick --locked --quiet

BIN="$PROJECT_ROOT/target/release-quick/qwick-memory"
[[ -x "$BIN" ]] || die "$STEP" "expected binary at $BIN"

SUBCOMMANDS=(
  save search list delete feedback doctor
  index-code symbol memory-for ast context walk
  conflicts supersedes prune gc install-hooks completions
)

{
  cat <<'HEADER'
# CLI reference

This page is **generated** by `scripts/regen-cli-docs.sh`. Do not edit by
hand — re-run the script and commit the result. Drift is enforced by
`scripts/cli-docs-check.sh` in the umbrella gate.

For the design rationale behind each command, see the
[design spec](superpowers/specs/2026-05-17-qwick-rust-agentic-rag-design.md).

## Global options

Every subcommand inherits two global flags:

| Flag | Description |
|---|---|
| `--json` | Emit machine-readable JSON instead of a human TTY view. |
| `--data-dir <DATA_DIR>` | Override the data root (defaults to `$HOME/.qwick-memory`). Honors the `QWICK_MEMORY_DATA_DIR` environment variable. |

Exit codes follow `sysexits.h`: `0` success, non-zero for usage / I/O /
data errors.

## Top-level help

```
HEADER

  "$BIN" --help

  echo '```'
  echo

  for sub in "${SUBCOMMANDS[@]}"; do
    echo "---"
    echo
    echo "## qwick-memory $sub"
    echo
    echo '```'
    "$BIN" "$sub" --help
    echo '```'
    echo
  done
} > "$OUT"

log_ok "$STEP" "wrote $OUT"
