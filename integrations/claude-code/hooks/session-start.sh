#!/usr/bin/env bash
# session-start.sh — SessionStart hook. Injects a compact digest of this repo's
# comemory memories so the agent starts with prior context. Strictly fail-soft:
# missing binary, empty result, or any non-zero exit prints nothing and exits 0,
# so the hook can never break a session.
set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
wrapper="${here}/../scripts/comemory.sh"
[ -x "$wrapper" ] || exit 0

# Plain (non-JSON) list, captured into a var: a later `head` truncation must
# not SIGPIPE-fail the producer (which `list | head` under pipefail would).
# This trusts `comemory list`'s plain format: one row per memory, no header
# (src/cli/list.rs), so an empty repo yields empty output. If `list` ever
# grows a header row, that header would surface as a fake memory line here —
# switch this to `list --json` + a `jq -r` projection at that point.
if ! out=$("$wrapper" list 2>/dev/null); then
    exit 0
fi

# Empty, or the missing-binary sentinel → nothing to inject.
case "$out" in
    '' | '{"comemory":"unavailable"'*) exit 0 ;;
esac

digest=$(printf '%s\n' "$out" | head -n 5)
[ -n "$digest" ] || exit 0

printf 'comemory — recalled memories for this repo:\n%s\n' "$digest"
