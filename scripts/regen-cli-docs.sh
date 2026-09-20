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

# Discover visible canonical names from clap's command rows. A fixed list can
# silently omit commands while the drift check still passes. Capture first so
# a failed --help cannot disappear inside process substitution on Bash 3.2.
CLI_HELP=$("$BIN" --help)
SUBCOMMANDS=$(printf '%s\n' "$CLI_HELP" | awk '
  /^Commands:$/ { in_commands = 1; next }
  in_commands && /^[^[:space:]]/ { exit }
  in_commands && /^  [^[:space:]]/ && $1 != "help" && $1 != "version" { print $1 }
')
[[ -n "$SUBCOMMANDS" ]] || die "$STEP" "no commands found in CLI help"

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

`comemory serve` also exposes almost every subcommand below as a versioned
JSON endpoint under `/api/v1` — see
[docs/guides/http-api.md](guides/http-api.md) for the route map, auth, and
job model.

## Top-level help

```
HEADER

  printf '%s\n' "$CLI_HELP"

  echo '```'
  echo

  while IFS= read -r sub; do
    echo "---"
    echo
    echo "## comemory $sub"
    echo
    echo '```'
    "$BIN" "$sub" --help
    echo '```'
    echo
  done <<< "$SUBCOMMANDS"
} | awk '{ sub(/[ \t]+$/, ""); print }' > "$OUT"

log_ok "$STEP" "wrote $OUT"
