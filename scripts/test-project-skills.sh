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
