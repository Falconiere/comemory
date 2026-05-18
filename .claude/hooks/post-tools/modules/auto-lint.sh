#!/usr/bin/env bash
# Auto-lint with clippy --fix on touched src/ or tests/ Rust files (silent on success).
set -uo pipefail

: "${tool_name:=}"; : "${input:=}"; : "${PROJECT_ROOT:=$(pwd)}"
[[ "$tool_name" != "Edit" && "$tool_name" != "Write" && "$tool_name" != "MultiEdit" ]] && exit 0

file_path=$(echo "$input" | jq -r '.tool_input.file_path // ""' 2>/dev/null || echo "")
[[ -z "$file_path" || ! -f "$file_path" ]] && exit 0
case "$file_path" in
  */src/*.rs|*/tests/*.rs)
    (cd "$PROJECT_ROOT" && cargo clippy --fix --allow-dirty --allow-staged --quiet -- -D warnings >/dev/null 2>&1) || true
    ;;
esac
exit 0
