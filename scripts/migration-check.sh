#!/usr/bin/env bash
# Immutability gate: every already-released migrations/*.sql file must be
# byte-identical to its content at the first release tag that shipped it.
# Migrations are append-only (see migrations/README.md) — store::migrate is
# marker-keyed and re-applies only what a given database has not yet seen,
# so editing an already-shipped file changes what a live user's database
# already applied without anything re-running it. A file in no tag yet is
# still in development and is skipped — that is the legitimate case of an
# unreleased migration.
#
# Two prefixes, one basename key. The directory moved from src/store/sql/ to
# migrations/ in the toolu-orm schema adoption (v0.29), so every tag up to
# v0.28.0 carries the files under LEGACY_DIR and every later tag under
# CURRENT_DIR. The union of ever-shipped files and the first-tag lookup both
# key on the basename and accept either prefix, so the move reads as a move,
# not as sixteen deletions plus sixteen unreleased files.
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
CURRENT_DIR="migrations"
LEGACY_DIR="src/store/sql"
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

mapfile -t SQL_FILES < <(git ls-files "$CURRENT_DIR/*.sql" | sort)

# The path a basename lived at in tag $1, if any: CURRENT_DIR first (the
# only place a post-move tag has it), then LEGACY_DIR. Prints nothing when
# the tag predates the file.
path_in_tag() {
  local tag="$1" base="$2"
  if git cat-file -e "$tag:$CURRENT_DIR/$base" 2>/dev/null; then
    printf '%s\n' "$CURRENT_DIR/$base"
  elif git cat-file -e "$tag:$LEGACY_DIR/$base" 2>/dev/null; then
    printf '%s\n' "$LEGACY_DIR/$base"
  fi
}

# `git ls-files` only walks paths that still exist, so DELETING a shipped
# migration file passes the modification loop below vacuously — nothing
# left to compare it against. That breaks a live user's database exactly as
# much as editing one does (their `schema_meta` marker still names the
# deleted file's key, and `store::migrate::list::MIGRATIONS` would fail to
# compile or the migration-integrity tests would fail first, but this
# immutability gate should catch the deletion directly rather than relying
# on those to notice it). Enumerate the UNION of shipped basenames across
# EVERY tag and BOTH prefixes — not just the newest tag: anchoring on a
# single tag heals itself the moment ANOTHER release is cut after a deletion
# lands, since the deleted file drops out of every later tag's tree too, and
# the gate would silently stop detecting the violation it exists to catch.
# Fail on any basename, from any tag, missing from the working tree.
declare -A EVER_SHIPPED=()
for t in "${TAGS[@]}"; do
  mapfile -t tagged_at_t < <(git ls-tree -r --name-only "$t" -- "$CURRENT_DIR" "$LEGACY_DIR" | grep '\.sql$' || true)
  for f in "${tagged_at_t[@]}"; do
    EVER_SHIPPED["$(basename "$f")"]=1
  done
done
mapfile -t EVER_SHIPPED_FILES < <(printf '%s\n' "${!EVER_SHIPPED[@]}" | sort)

deleted=0
for base in "${EVER_SHIPPED_FILES[@]}"; do
  if [[ ! -f "$CURRENT_DIR/$base" ]]; then
    log_err "$STEP" \
      "$CURRENT_DIR/$base was shipped in a prior release but is missing from the working tree — migrations \
are append-only and never deleted once released"
    deleted=1
  fi
done

modified=0
for f in "${SQL_FILES[@]}"; do
  base="$(basename "$f")"
  first_tag=""
  first_path=""
  for t in "${TAGS[@]}"; do
    first_path="$(path_in_tag "$t" "$base")"
    if [[ -n "$first_path" ]]; then
      first_tag="$t"
      break
    fi
  done

  if [[ -z "$first_tag" ]]; then
    log_info "$STEP" "$f: not yet released, skipping"
    continue
  fi

  if [[ ! -f "$f" ]]; then
    # `git ls-files` still lists a tracked file deleted from the working tree
    # without `git rm`; the deletion loop above already reported it — do not
    # also mislabel it "differs from its first release" here.
    continue
  fi

  if diff -q <(git show "$first_tag:$first_path") "$f" >/dev/null 2>&1; then
    log_info "$STEP" "$f: unchanged since $first_tag ($first_path)"
  else
    log_err "$STEP" \
      "$f differs from its first release ($first_tag, $first_path) — migrations are immutable once shipped; append a new numbered file instead"
    modified=1
  fi
done

if (( deleted && modified )); then
  die "$STEP" "one or more shipped migrations were deleted, and one or more were modified"
elif (( deleted )); then
  die "$STEP" "one or more shipped migrations were deleted from the working tree"
elif (( modified )); then
  die "$STEP" "one or more shipped migrations were modified"
fi

log_ok "$STEP" "${#SQL_FILES[@]} migration file(s) verified immutable"
