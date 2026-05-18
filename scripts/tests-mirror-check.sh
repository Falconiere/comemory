#!/usr/bin/env bash
# Every src/<path>/<name>.rs must have a matching tests/<path>/<name>.rs.
# Exceptions: lib.rs, main.rs, prelude.rs, mod.rs, errors.rs, files inside src/cli/ (covered by tests/cli.rs).
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
  mirror="tests/${base}"
  if [[ ! -f "$mirror" ]]; then
    missing+=("$mirror  (mirrors $f)")
  fi
done < <(git ls-files -z 'src/*.rs')

if (( ${#missing[@]} > 0 )); then
  log_err "tests-mirror-check" "missing test files:"
  printf "  %s\n" "${missing[@]}" >&2
  exit 1
fi
log_ok "tests-mirror-check" "every src file has a test mirror"
