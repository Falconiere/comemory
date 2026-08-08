#!/usr/bin/env bash
# Immutability gate: every already-released src/store/sql/*.sql file must be
# byte-identical to its content at the first release tag that shipped it.
# Migrations are append-only (see src/store/sql/README.md) — store::migrate
# is marker-keyed and re-applies only what a given database has not yet seen,
# so editing an already-shipped file changes what a live user's database
# already applied without anything re-running it. A file in no tag yet is
# still in development and is skipped — that is the legitimate case of an
# unreleased migration.
#
# All tags are cut from main by release-plz (docs/release.md), so "first tag
# containing the file" is well-defined. Tags are sorted by creatordate, NOT
# alphabetically — "v0.10.0" sorts before "v0.2.0" lexically, which would
# pick the wrong (later) release as the anchor.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

STEP="migration-check"
cd "$PROJECT_ROOT"
require_cmd git

mapfile -t TAGS < <(git tag --list --sort=creatordate)

# A clone with no visible tags cannot tell a shipped migration from an
# unreleased one, so every file would fall into the skip branch below and
# this gate would pass vacuously. That is exactly the state a shallow CI
# checkout starts in (actions/checkout defaults to depth 1 with tags off),
# so this must fail loudly rather than silently protect nothing.
if (( ${#TAGS[@]} == 0 )); then
  log_err "$STEP" \
    "no git tags visible in this clone — cannot verify migration immutability. Run: git fetch --tags --unshallow"
  exit 1
fi

mapfile -t SQL_FILES < <(git ls-files 'src/store/sql/*.sql' | sort)

failed=0
for f in "${SQL_FILES[@]}"; do
  first_tag=""
  for t in "${TAGS[@]}"; do
    if git cat-file -e "$t:$f" 2>/dev/null; then
      first_tag="$t"
      break
    fi
  done

  if [[ -z "$first_tag" ]]; then
    log_info "$STEP" "$f: not yet released, skipping"
    continue
  fi

  if diff -q <(git show "$first_tag:$f") "$f" >/dev/null 2>&1; then
    log_info "$STEP" "$f: unchanged since $first_tag"
  else
    log_err "$STEP" \
      "$f differs from its first release ($first_tag) — migrations are immutable once shipped; append a new numbered file instead"
    failed=1
  fi
done

if (( failed )); then
  die "$STEP" "one or more shipped migrations were modified"
fi

log_ok "$STEP" "${#SQL_FILES[@]} migration file(s) verified immutable"
