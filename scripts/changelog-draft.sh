#!/usr/bin/env bash
# scripts/changelog-draft.sh — bucket conventional commits since <ref> into a
# Keep-a-Changelog markdown section. Reads <ref>..HEAD from git log, parses
# the subject line, and prints to stdout. Paste the result under
# "## [Unreleased]" in CHANGELOG.md, edit the bucket names, then move it
# under a dated heading.
#
# Usage: bash scripts/changelog-draft.sh [<since-ref>]
#   <since-ref> defaults to the most recent semver tag (RCs excluded).
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

step="changelog-draft"
raw_since="${1:-}"

if [[ -z "$raw_since" ]]; then
  since_no_v="$(git -C "$PROJECT_ROOT" tag --sort=-v:refname \
    | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' \
    | head -1 \
    | sed 's/^v//')"
  if [[ -z "$since_no_v" ]]; then
    die "$step" "no semver tag found; pass <since-ref> explicitly (e.g. v0.10.0)"
  fi
  log_info "$step" "using since-ref: v${since_no_v} (latest non-RC semver tag)"
else
  since_no_v="${raw_since#v}"
fi

# %s strips the (epoch, author, refs) noise and keeps just the subject.
mapfile -t subjects < <(git -C "$PROJECT_ROOT" log "v${since_no_v}..HEAD" --pretty='%s' || true)

if [[ ${#subjects[@]} -eq 0 ]]; then
  die "$step" "no commits found in v${since_no_v}..HEAD"
fi

# Buckets (preserved order in the output, matching Keep-a-Changelog 1.1.0).
declare -A buckets=(
  ["### Added"]=""
  ["### Changed"]=""
  ["### BREAKING"]=""
  ["### Fixed"]=""
  ["### Removed"]=""
  ["### Security"]=""
  ["### Internal"]=""
)
declare -a non_conventional=()

add_line() { buckets["$1"]+="$2"$'\n'; }

parse_subject() {
  local subject="$1"
  # Conventional commit: TYPE[(scope)][!]: DESCRIPTION
  # Split on the first ":" into type_part and rest. Strip optional scope
  # and bang from type_part with pure parameter expansion (no nested
  # regex groups, which `[[ =~ ]]` parses awkwardly).
  local type_part="${subject%%:*}"
  if [[ -z "$type_part" || "$type_part" == "$subject" ]]; then
    # No ":" found — not a conventional commit.
    non_conventional+=("$subject")
    return
  fi
  local rest="${subject#*: }"
  local bang=""
  if [[ "$type_part" == *"!"* ]]; then
    bang="!"
    type_part="${type_part%!}"
  fi
  local type="${type_part%(*}"

  local line="- ${rest}"
  if [[ -n "$bang" ]]; then
    add_line "### BREAKING" "$line"
    return
  fi
  case "$type" in
    feat)                    add_line "### Added"    "$line" ;;
    fix)                      add_line "### Fixed"    "$line" ;;
    refactor|perf|style)      add_line "### Changed"  "$line" ;;
    revert)                   add_line "### Removed"  "$line" ;;
    docs|chore|ci|test|build) add_line "### Internal" "$line" ;;
    *)
      # Unknown conventional type — surface with a marker so the maintainer
      # can spot it in the rendered output.
      add_line "### Internal" "$line  _[unknown-type:${type}]_"
      ;;
  esac
}

for s in "${subjects[@]}"; do
  parse_subject "$s"
done

cat <<HDR
## [Unreleased]

_Unreleased changes since v${since_no_v}._

HDR

for bucket in "### Added" "### Changed" "### BREAKING" "### Fixed" "### Removed" "### Security" "### Internal"; do
  printf '%s\n\n' "$bucket"
  if [[ -n "${buckets[$bucket]}" ]]; then
    printf '%s' "${buckets[$bucket]}"
  else
    printf '_(empty)_\n'
  fi
  printf '\n'
done

if [[ ${#non_conventional[@]} -gt 0 ]]; then
  cat <<'TAIL'
### Notes

The following commits did not start with a conventional prefix; review and
move into the right bucket, or add a prefix and re-run `just changelog`:

TAIL
  for s in "${non_conventional[@]}"; do
    printf -- '- %s\n' "$s"
  done
fi

log_ok "$step" "drafted from v${since_no_v}..HEAD (${#subjects[@]} commits)"
