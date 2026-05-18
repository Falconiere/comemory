#!/usr/bin/env bash
# Rules for Bash/Shell tool invocations.
#   1. cargo-only — block npm/bun/pip/uv (none are part of this Rust project)
#   2. Block destructive commands (rm -rf, git push --force, git reset --hard, chmod -R 777)
#   3. Block bypass flags (--no-verify, --no-gpg-sign)
#   4. Block direct rustfmt/clippy invocation — must go through scripts/ or just

: "${tool_name:=}"
: "${input:=}"

[[ "$tool_name" != "Bash" && "$tool_name" != "Shell" ]] && exit 0
HOOK_LIB="$(cd "$(dirname "$0")/../.." && pwd)/lib/common.sh"
# shellcheck source=../../lib/common.sh
source "$HOOK_LIB"

command=$(echo "$input" | jq -r '.tool_input.command // ""')
[[ -z "$command" ]] && exit 0

cmd_only=$(echo "$command" | sed '/<<['"'"'"]*EOF['"'"'"]*$/,/^EOF$/d')

if echo "$cmd_only" | grep -qE '(^|[[:space:]]|&&|\|\||;|`|\()(npm|npx|yarn|pnpm|bun|bunx|pip|uv|poetry)[[:space:]]'; then
  deny_pre "qwick is a Rust project — use cargo / just / scripts/* instead of npm|bun|pip|uv."
  exit 0
fi

if echo "$cmd_only" | grep -qE '(^|[[:space:]]|&&|\|\||;)(rm -rf|git push.*--force|git reset --hard|git checkout \.|chmod -R 777)'; then
  deny_pre "Destructive command blocked: $cmd_only"
  exit 0
fi

if echo "$cmd_only" | grep -qE '(--no-verify|--no-gpg-sign)'; then
  deny_pre "--no-verify / --no-gpg-sign forbidden. Rules cannot be bypassed."
  exit 0
fi

if echo "$cmd_only" | grep -qE '(^|[[:space:]]|&&|\|\||;)(rustfmt|cargo[[:space:]]+fmt[[:space:]]|cargo[[:space:]]+clippy[[:space:]])'; then
  if ! echo "$cmd_only" | grep -qE '(scripts/|just[[:space:]]|lefthook|post-tools|pre-tools)'; then
    deny_pre "Run quality gates via scripts/* or 'just check' — do not invoke rustfmt/clippy directly."
    exit 0
  fi
fi
exit 0
