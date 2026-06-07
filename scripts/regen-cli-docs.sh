#!/usr/bin/env bash
# Regenerate docs/cli-reference.md from `comemory <cmd> --help` output.
# This is the single source of truth for the CLI reference page.

set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

STEP="regen-cli-docs"

OUT="${1:-$PROJECT_ROOT/docs/cli-reference.md}"

log_info "$STEP" "building release-quick binary"
run_cargo build --profile release-quick --locked --quiet

BIN="$PROJECT_ROOT/target/release-quick/comemory"
[[ -x "$BIN" ]] || die "$STEP" "expected binary at $BIN"

SUBCOMMANDS=(
  save search list delete feedback doctor
  index-code symbol memory-for ast context walk
  conflicts supersedes prune gc install-hooks completions
)

# Nested subcommands: each entry is "parent sub" (space-separated).
NESTED_SUBCOMMANDS=(
  "graph serve"
)

{
  cat <<'HEADER'
# CLI reference

This page is **generated** by `scripts/regen-cli-docs.sh`. Do not edit by
hand — re-run the script and commit the result. Drift is enforced by
`scripts/cli-docs-check.sh` in the umbrella gate.

## Global options

Every subcommand inherits two global flags:

| Flag | Description |
|---|---|
| `--json` | Emit machine-readable JSON instead of a human TTY view. |
| `--data-dir <DATA_DIR>` | Override the data root (defaults to `$HOME/.comemory`). Honors the `COMEMORY_DATA_DIR` environment variable. |

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
    echo "## comemory $sub"
    echo
    echo '```'
    "$BIN" "$sub" --help
    echo '```'
    echo
  done

  # Emit parent-level help then each nested subcommand.
  # emitted_parents is a colon-delimited string (bash 3 compatible).
  emitted_parents=""
  for nested in "${NESTED_SUBCOMMANDS[@]}"; do
    parent="${nested%% *}"
    if [[ ":${emitted_parents}:" != *":${parent}:"* ]]; then
      echo "---"
      echo
      echo "## comemory $parent"
      echo
      echo '```'
      "$BIN" "$parent" --help
      echo '```'
      echo
      emitted_parents="${emitted_parents}:${parent}"
    fi
    echo "---"
    echo
    # shellcheck disable=SC2086
    echo "## comemory $nested"
    echo
    echo '```'
    # shellcheck disable=SC2086
    "$BIN" $nested --help
    echo '```'
    echo
  done
} > "$OUT"

log_ok "$STEP" "wrote $OUT"
