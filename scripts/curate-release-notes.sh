#!/usr/bin/env bash
# scripts/curate-release-notes.sh — extract the CHANGELOG.md section for a
# given version. Used by .github/workflows/release-finalize.yml to set
# the curated GitHub Release body (replacing cargo-dist's auto-blob).
#
# Usage: bash scripts/curate-release-notes.sh <version>
#   <version> is the bare semver (e.g. 0.10.0), no leading 'v'.
#
# Output: the markdown body of the matching `## [<version>]` section, with
# the heading line included, on stdout. If no matching section is found,
# a friendly fallback (with the GitHub compare URL) is emitted instead.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"

step="curate-release-notes"
ver="${1:-}"
if [[ -z "$ver" ]]; then
  die "$step" "usage: bash $0 <version>  (e.g. 0.10.0)"
fi

changelog="$PROJECT_ROOT/CHANGELOG.md"
if [[ ! -f "$changelog" ]]; then
  die "$step" "CHANGELOG.md not found at $changelog"
fi

# Extract the section between the matching `## [<ver>]` heading and the
# next `## [` heading (or EOF). awk with index() (not regex) because
# the target contains literal `[` / `]` which awk's ERE treats specially.
section="$(awk -v target="## [${ver}]" '
  index($0, target) == 1 { in_section = 1; print; next }
  in_section && /^## \[/ { exit }
  in_section { print }
' "$changelog")"

# Trim trailing blank lines so the output is clean.
section="$(printf '%s\n' "$section" | awk '
  { lines[NR] = $0 }
  END {
    end = NR
    while (end > 0 && lines[end] == "") end--
    for (i = 1; i <= end; i++) print lines[i]
  }
')"

if [[ -n "$section" ]]; then
  printf '%s\n' "$section"
  log_ok "$step" "extracted section for $ver"
  exit 0
fi

# Fallback: emit a friendly placeholder with the GitHub compare URL.
# Find the previous tag (if any) so the compare link is useful.
prev_tag="$(grep -oE '^\[[0-9]+\.[0-9]+\.[0-9]+(\.[0-9]+)*\]:' "$changelog" \
  | sed -E 's/^\[([^]]+)\]:/\1/' \
  | awk -v v="$ver" '$0 == v { found = 1; next } found { print "v" $0; exit }' \
  || true)"
prev_arg=""
if [[ -n "$prev_tag" ]]; then
  prev_arg="$prev_tag...v$ver"
else
  prev_arg="v$ver"
fi

cat <<FALLBACK
## [${ver}](https://github.com/Falconiere/comemory/releases/tag/v${ver})

See the commit log for the full set of changes since the previous release:

<https://github.com/Falconiere/comemory/compare/${prev_arg}>
FALLBACK

log_info "$step" "no CHANGELOG.md section for $ver; emitted fallback"
