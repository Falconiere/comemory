#!/usr/bin/env bash
# Fail if any tracked file under src/ or scripts/ exceeds 500 lines.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

cd "$PROJECT_ROOT"
LIMIT=500
oversized=()

while IFS= read -r -d '' f; do
  lines=$(wc -l < "$f")
  if (( lines > LIMIT )); then
    oversized+=("$f ($lines lines)")
  fi
done < <(git ls-files -z 'src/*.rs' 'scripts/*.sh' 'scripts/**/*.sh')

if (( ${#oversized[@]} > 0 )); then
  log_err "module-size-check" "file(s) exceed $LIMIT lines:"
  printf "  %s\n" "${oversized[@]}" >&2
  printf "\nSplit them into smaller, focused modules.\n" >&2
  exit 1
fi
log_ok "module-size-check" "all modules within $LIMIT lines"
