#!/usr/bin/env bash
# Block forbidden patterns anywhere except docs/, tests/, scripts/no-bypass-check.sh, and inside #[cfg(test)] blocks.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

cd "$PROJECT_ROOT"

declare -a NAMES=(
  "allow-attr"
  "clippy-allow-comment"
  "unwrap-in-src"
  "expect-without-msg"
  "println-in-src"
  "eprintln-in-src"
  "todo-macro"
  "unimplemented-macro"
  "unsafe-without-safety-comment"
  "panic-in-src"
)

declare -a REGEXES=(
  '#\[allow\('
  '//\s*clippy::allow'
  '\.unwrap\(\)'
  '\.expect\(\s*\)'
  '\bprintln!'
  '\beprintln!'
  '\btodo!\('
  '\bunimplemented!\('
  '\bunsafe\s*\{'
  '\bpanic!\('
)

EXCLUDES=(
  ":(exclude)scripts/no-bypass-check.sh"
)

fail=0
for i in "${!NAMES[@]}"; do
  name="${NAMES[$i]}"
  pattern="${REGEXES[$i]}"
  hits=$(git grep -nE "$pattern" -- 'src/*.rs' "${EXCLUDES[@]}" 2>/dev/null || true)

  # Special case: unsafe { is allowed when preceded within 3 lines by `// SAFETY:`
  if [[ "$name" == "unsafe-without-safety-comment" && -n "$hits" ]]; then
    filtered=""
    while IFS= read -r line; do
      file="${line%%:*}"
      lineno="${line#*:}"; lineno="${lineno%%:*}"
      start=$((lineno > 3 ? lineno - 3 : 1))
      window=$(sed -n "${start},${lineno}p" "$file" 2>/dev/null || true)
      if ! echo "$window" | grep -q "SAFETY:"; then
        filtered+="$line"$'\n'
      fi
    done <<< "$hits"
    hits="${filtered%$'\n'}"
  fi

  if [[ -n "$hits" ]]; then
    log_err "no-bypass-check" "forbidden pattern '$name':"
    printf "%s\n" "$hits" >&2
    fail=1
  fi
done

if (( fail != 0 )); then
  printf "\nRules cannot be bypassed. Fix the root cause.\n" >&2
  exit 1
fi
log_ok "no-bypass-check" "no forbidden patterns"
