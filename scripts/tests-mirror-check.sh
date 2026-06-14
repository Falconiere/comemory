#!/usr/bin/env bash
# Flat dunder mirror check: every src/<path>/<name>.rs must have a matching
# tests/<path-with-slashes-as-dunder>__<name>.rs (e.g. src/store/tokenizer/split.rs
# → tests/store__tokenizer__split.rs).
# Exceptions: lib.rs, main.rs, prelude.rs, mod.rs, errors.rs, files inside src/cli/.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

cd "$PROJECT_ROOT"
missing=()

while IFS= read -r -d '' f; do
  base="${f#src/}"
  name="$(basename "$base")"
  case "$name" in
    lib.rs|main.rs|prelude.rs|mod.rs|errors.rs) continue ;;
  esac
  case "$base" in
    cli/*) continue ;;
  esac
  # Flat dunder mapping: replace every '/' with '__', drop the .rs, re-add .rs
  base_no_ext="${base%.rs}"
  flat_name="${base_no_ext//\//__}"
  flat="tests/${flat_name}.rs"
  if [[ ! -f "$flat" ]]; then
    missing+=("$flat  (mirrors $f)")
  fi
done < <(git ls-files -z 'src/*.rs')

if (( ${#missing[@]} > 0 )); then
  log_err "tests-mirror-check" "missing flat-dunder test mirrors:"
  printf "  %s\n" "${missing[@]}" >&2
  printf "\nExpected layout: src/a/b/c.rs -> tests/a__b__c.rs\n" >&2
  exit 1
fi
log_ok "tests-mirror-check" "every src file has a flat-dunder test mirror"
