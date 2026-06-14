#!/usr/bin/env bash
# Two separate size limits:
#   src/*.rs  — 300 CODE lines (blank lines and comment-only lines excluded)
#   scripts/*.sh + scripts/lib/*.sh — 500 TOTAL lines (scripts are not lang-quality-governed)
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

cd "$PROJECT_ROOT"
SRC_LIMIT=300
SCRIPT_LIMIT=500
oversized=()

# Count code lines in a Rust file: exclude blank lines and single-line comment lines.
# Block comments (/* ... */) are tracked across lines with a flag.
count_code_lines() {
  local file="$1"
  local count=0
  local in_block=0
  while IFS= read -r line; do
    stripped="${line#"${line%%[![:space:]]*}"}"  # ltrim
    if (( in_block )); then
      if [[ "$stripped" == *'*/'* ]]; then
        in_block=0
      fi
      continue
    fi
    [[ -z "$stripped" ]] && continue           # blank line
    [[ "$stripped" == '//'* ]] && continue      # comment-only line
    if [[ "$stripped" == '/*'* ]]; then
      if [[ "$stripped" != *'*/'* ]]; then
        in_block=1
      fi
      continue
    fi
    (( count++ )) || true
  done < "$file"
  echo "$count"
}

while IFS= read -r -d '' f; do
  code_lines=$(count_code_lines "$f")
  if (( code_lines > SRC_LIMIT )); then
    oversized+=("$f ($code_lines code lines, limit $SRC_LIMIT)")
  fi
done < <(git ls-files -z 'src/*.rs')

while IFS= read -r -d '' f; do
  total=$(wc -l < "$f")
  if (( total > SCRIPT_LIMIT )); then
    oversized+=("$f ($total lines, limit $SCRIPT_LIMIT)")
  fi
done < <(git ls-files -z 'scripts/*.sh' 'scripts/lib/*.sh')

if (( ${#oversized[@]} > 0 )); then
  log_err "module-size-check" "file(s) exceed size limit:"
  printf "  %s\n" "${oversized[@]}" >&2
  printf "\nSplit src/ files at 300 code lines; scripts at 500 total lines.\n" >&2
  exit 1
fi
log_ok "module-size-check" "all modules within size limits (src: ${SRC_LIMIT} code lines, scripts: ${SCRIPT_LIMIT} total lines)"
