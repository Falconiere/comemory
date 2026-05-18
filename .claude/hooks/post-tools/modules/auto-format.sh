#!/usr/bin/env bash
# Auto-format touched files (silent on success). Rust via rustfmt, TOML via taplo (if present).
set -uo pipefail

: "${tool_name:=}"; : "${input:=}"; : "${PROJECT_ROOT:=$(pwd)}"
[[ "$tool_name" != "Edit" && "$tool_name" != "Write" && "$tool_name" != "MultiEdit" ]] && exit 0

file_path=$(echo "$input" | jq -r '.tool_input.file_path // ""' 2>/dev/null || echo "")
[[ -z "$file_path" || ! -f "$file_path" ]] && exit 0

case "$file_path" in
  *.rs)
    (cd "$PROJECT_ROOT" && rustfmt --emit=files --edition 2021 "$file_path" >/dev/null 2>&1) || true
    ;;
  *.toml)
    (cd "$PROJECT_ROOT" && command -v taplo >/dev/null 2>&1 && taplo fmt "$file_path" >/dev/null 2>&1) || true
    ;;
esac
exit 0
