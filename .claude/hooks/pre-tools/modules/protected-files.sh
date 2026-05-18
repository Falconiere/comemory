#!/usr/bin/env bash
# Block edits to vendored/build/protected paths.
set -uo pipefail

: "${tool_name:=}"
: "${input:=}"

[[ "$tool_name" != "Edit" && "$tool_name" != "Write" && "$tool_name" != "MultiEdit" ]] && exit 0
HOOK_LIB="$(cd "$(dirname "$0")/../.." && pwd)/lib/common.sh"
# shellcheck source=../../lib/common.sh
source "$HOOK_LIB"

file_path=$(echo "$input" | jq -r '.tool_input.file_path // ""')
[[ -z "$file_path" ]] && exit 0

case "$file_path" in
  */target/*|target/*|*/Cargo.lock|Cargo.lock)
    deny_pre "$file_path is a build artifact — do not edit by hand."
    exit 0 ;;
  */deny.toml|deny.toml|*/lefthook.yml|lefthook.yml|*/rustfmt.toml|rustfmt.toml|*/clippy.toml|clippy.toml|*/typos.toml|typos.toml|*/.github/workflows/ci.yml|.github/workflows/ci.yml)
    deny_pre "$file_path is a protected config — requires explicit user request to edit."
    exit 0 ;;
esac
exit 0
