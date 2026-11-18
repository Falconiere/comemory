#!/usr/bin/env bash
# Exercise immutability against real SQL files and disposable release history.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/scripts/lib" "$work/migrations" "$work/src/store/sql"
cp "$ROOT/scripts/migration-check.sh" "$work/scripts/"
cp "$ROOT/scripts/lib/common.sh" "$work/scripts/lib/"
cp "$ROOT/migrations/README.md" "$work/README.md"
cd "$work"
git init -q
git config user.name 'Migration gate test'
git config user.email 'migration-test@example.invalid'

release() {
  git add -A
  GIT_AUTHOR_DATE="$2" GIT_COMMITTER_DATE="$2" \
    git -c core.hooksPath=/dev/null -c commit.gpgsign=false commit -qm "$1"
  git -c tag.gpgsign=false tag "$1"
}

check() {
  local status=0
  bash scripts/migration-check.sh >"$work/result" 2>&1 || status=$?
  if [[ "$status" != "$1" ]] || ! grep -Fq "$2" "$work/result"; then
    cat "$work/result" >&2
    printf 'FAIL: expected exit %s and %s; got %s\n' "$1" "$2" "$status" >&2
    exit 1
  fi
}

check 1 'no git tags visible'
release v0.1.0 2020-01-01T00:00:00Z
cp "$ROOT/migrations/0001_schema_meta.sql" migrations/
git add migrations/
check 0 'not yet released, skipping'

git mv migrations/0001_schema_meta.sql src/store/sql/
release v0.2.0 2020-01-02T00:00:00Z
git mv src/store/sql/0001_schema_meta.sql migrations/
check 0 'unchanged since v0.2.0 (src/store/sql/0001_schema_meta.sql)'

# Both prefixes in the first release: current wins even with different legacy SQL.
cp "$ROOT/migrations/0002_v2_tables.sql" migrations/
cp "$ROOT/migrations/0003_stats_tables.sql" src/store/sql/0002_v2_tables.sql
release v0.10.0 2020-01-03T00:00:00Z
check 0 'unchanged since v0.10.0 (migrations/0002_v2_tables.sql)'

# A later tag containing changed bytes must not replace the original anchor.
cp "$ROOT/migrations/0002_v2_tables.sql" migrations/0001_schema_meta.sql
release v0.11.0 2020-01-04T00:00:00Z
check 1 'differs from its first release (v0.2.0, src/store/sql/0001_schema_meta.sql)'

# A deletion must remain detectable even after a release stops listing the file.
cp "$ROOT/migrations/0001_schema_meta.sql" migrations/
git rm -q migrations/0002_v2_tables.sql src/store/sql/0002_v2_tables.sql
release v0.12.0 2020-01-05T00:00:00Z
check 1 'migrations/0002_v2_tables.sql was shipped in a prior release but is missing'
printf 'PASS: migration history, moves, path priority, modifications and deletions\n'
