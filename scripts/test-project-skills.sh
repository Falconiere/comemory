#!/usr/bin/env bash
# Real filesystem lifecycle check for the bundled project-skills wrapper.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
WRAPPER="$ROOT/integrations/agent/skills/project-skills/scripts/skills.sh"
TMP_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/project skills.XXXXXX")
trap 'rm -rf "$TMP_ROOT"' EXIT
REPO="$TMP_ROOT/real repository"
mkdir -p "$REPO"
git -C "$REPO" init -q

BODY='## When to Use

Use this procedure.

## Procedure

Run it.

## Pitfalls

Do not skip verification.

## Verification

Inspect the result.'

cd "$REPO"
printf '%s\n' "$BODY" | "$WRAPPER" create release-check --description 'Verify release outputs'
"$WRAPPER" archive release-check
[ -f "$REPO/.toolu/skills/.archive/release-check/SKILL.md" ]
"$WRAPPER" restore release-check
[ -f "$REPO/.toolu/skills/release-check/SKILL.md" ]
"$WRAPPER" pin release-check
jq -e '."release-check".pinned == true' "$REPO/.toolu/skills/.usage.json" >/dev/null
"$WRAPPER" unpin release-check
jq -e '."release-check".pinned == false' "$REPO/.toolu/skills/.usage.json" >/dev/null

mkdir -p "$REPO/.toolu/skills/imported-procedure"
printf '%s\n' '---' 'name: imported-procedure' 'description: Imported procedure' '---' "$BODY" \
  >"$REPO/.toolu/skills/imported-procedure/SKILL.md"
"$WRAPPER" adopt imported-procedure
jq -e '."imported-procedure".origin == "agent"' "$REPO/.toolu/skills/.usage.json" >/dev/null

# Both skills idle for decades: the one a .claude/skills link exposes as a slash
# command stays, the unlinked one is archived (the control proving curate ran).
mkdir -p "$REPO/.claude/skills"
ln -s ../../.toolu/skills/imported-procedure "$REPO/.claude/skills/imported-procedure"
USAGE="$REPO/.toolu/skills/.usage.json"
jq '(."imported-procedure", ."release-check").last_used_at = "2000-01-01T00:00:00Z"' "$USAGE" >"$USAGE.tmp"
mv "$USAGE.tmp" "$USAGE"
"$WRAPPER" curate
[ -f "$REPO/.toolu/skills/imported-procedure/SKILL.md" ]
[ -f "$REPO/.toolu/skills/.archive/release-check/SKILL.md" ]
