#!/usr/bin/env bash
# Token-clone ratchet over production Rust + scripts/, using a PINNED jscpd
# build (https://github.com/kucherenko/jscpd). Complements
# scripts/dup-check.sh (similarity-rs function/method near-dups): jscpd finds
# copy-pasted token blocks, including in shell that similarity-rs cannot parse.
#
# WHY THE VERSION IS PINNED. Clone counts move between jscpd releases (token
# rules, defaults). An unpinned binary would make jscpd-baseline.txt a
# measurement of the runner. Mismatch is a HARD FAILURE everywhere.
#
# .github/workflows/test.yml DERIVES the version it installs from the exact
# assignment line below. Keep it on one line, spelled exactly as it is.
#
# NOT in scripts/check-all.sh's GATES — needs a separately installed tool;
# grouped with the other tool-pinned gates in the justfile `qa` recipe and CI.
set -euo pipefail

JSCPD_VERSION="5.4.2"
# Hard ceiling on duplicated *lines* percentage (jscpd statistics.total.percentage).
# Same 5% residual policy as scripts/dup-check.sh's unledgered budget — a
# different detector, same project ceiling. Count ratchet still tightens.
JSCPD_PCT_MAX=5

HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"
cd "$PROJECT_ROOT"

STEP="jscpd-check"
INSTALL_HINT="cargo install jscpd --version $JSCPD_VERSION --locked"
BASELINE_FILE="$PROJECT_ROOT/jscpd-baseline.txt"

JSCPD_BIN="$(command -v jscpd 2>/dev/null || true)"
if [[ -z "$JSCPD_BIN" ]]; then
  if [[ -n "${CI:-}" ]]; then
    log_err "$STEP" \
      "jscpd is required in CI but is not on PATH — install it with: $INSTALL_HINT"
    exit 1
  fi
  printf '%s[%s]%s %s[warn]%s the jscpd gate DID NOT RUN — jscpd is not installed\n' \
    "$C_DIM" "$STEP" "$C_RST" "$C_YLW" "$C_RST" >&2
  printf '%s[%s]%s %s[warn]%s install the pinned build to run it: %s\n' \
    "$C_DIM" "$STEP" "$C_RST" "$C_YLW" "$C_RST" "$INSTALL_HINT" >&2
  exit 0
fi

version_output="$("$JSCPD_BIN" --version 2>/dev/null || true)"
installed_version="$(printf '%s\n' "$version_output" | awk 'NF { print $NF; exit }')"
if [[ -z "$installed_version" ]]; then
  log_err "$STEP" \
    "could not read the installed jscpd version (pinned to $JSCPD_VERSION); install with: $INSTALL_HINT"
  exit 1
fi
if [[ "$installed_version" != "$JSCPD_VERSION" ]]; then
  log_err "$STEP" \
    "jscpd $installed_version is on PATH but this gate is pinned to $JSCPD_VERSION — counts from another build are not comparable to jscpd-baseline.txt; install with: $INSTALL_HINT"
  exit 1
fi

if [[ ! -f "$BASELINE_FILE" ]]; then
  log_err "$STEP" "missing $BASELINE_FILE (baseline ratchet file)"
  exit 1
fi
baseline_count="$(tr -d '[:space:]' < "$BASELINE_FILE")"
if ! [[ "$baseline_count" =~ ^[0-9]+$ ]]; then
  log_err "$STEP" "$BASELINE_FILE must contain a single integer clone count"
  exit 1
fi

# Directories (not a hand-built file list): jscpd silently drops a few
# empty/unparseable paths when given hundreds of explicit files, which made an
# exact file-count anti-vacuous check vacuous itself. Ignore production-test
# trees the same way scripts/dup-check.sh excludes /tests/.
report_dir="$(mktemp -d -t comemory-jscpd.XXXXXX)"
trap 'rm -rf "$report_dir"' EXIT

# silent + json: we own the pass/fail signal via the baseline comparison.
set +e
"$JSCPD_BIN" src scripts \
  --format rust,bash \
  --ignore '**/tests/**,**/proptest-regressions/**' \
  --mode mild \
  --reporters json,silent \
  --output "$report_dir" \
  >/dev/null 2>"$report_dir/stderr.txt"
jscpd_ec=$?
set -e
# jscpd exits 0 even when clones exist; non-zero is a tool failure.
if (( jscpd_ec != 0 )); then
  log_err "$STEP" "jscpd exited $jscpd_ec; see $report_dir/stderr.txt"
  trap - EXIT
  exit 1
fi

report_json="$report_dir/jscpd-report.json"
if [[ ! -f "$report_json" ]]; then
  log_err "$STEP" "jscpd did not write $report_json; see $report_dir/stderr.txt"
  trap - EXIT
  exit 1
fi

parsed="$(python3 -c '
import json, sys
d = json.load(open(sys.argv[1]))
total = d["statistics"]["total"]
# percentage is a float (duplicated lines / total lines * 100).
print("%s %s %.6f" % (total["clones"], total["sources"], float(total["percentage"])))
' "$report_json")"
# clones sources pct
current_count="$(printf '%s\n' "$parsed" | awk '{ print $1 }')"
sources_count="$(printf '%s\n' "$parsed" | awk '{ print $2 }')"
lines_pct="$(printf '%s\n' "$parsed" | awk '{ print $3 }')"
if ! [[ "$current_count" =~ ^[0-9]+$ && "$sources_count" =~ ^[0-9]+$ ]]; then
  log_err "$STEP" "could not parse jscpd statistics from $report_json"
  trap - EXIT
  exit 1
fi

# ANTI-VACUOUS: a scan that ingested nothing would report 0 sources and
# "pass" a zero baseline. Floor is well below a healthy tree (~700+ today).
if (( sources_count < 100 )); then
  log_err "$STEP" \
    "jscpd analyzed only $sources_count sources — scan looks vacuous"
  trap - EXIT
  exit 1
fi

log_info "$STEP" \
  "clones $current_count/$baseline_count, duplicated-lines ${lines_pct}% (ceiling ${JSCPD_PCT_MAX}%) over $sources_count sources (jscpd $JSCPD_VERSION)"

failed=0
# bash cannot compare floats; use awk.
if awk -v p="$lines_pct" -v max="$JSCPD_PCT_MAX" 'BEGIN { exit !(p > max) }'; then
  log_err "$STEP" \
    "duplicated-lines percentage $lines_pct exceeds the ${JSCPD_PCT_MAX}% ceiling"
  failed=1
fi
if (( current_count > baseline_count )); then
  log_err "$STEP" \
    "clone count $current_count exceeds baseline $baseline_count (docs/dup-debt.md); see $report_json"
  failed=1
fi
if (( failed != 0 )); then
  trap - EXIT
  exit 1
fi
if (( current_count < baseline_count )); then
  log_info "$STEP" \
    "clone count $current_count is below baseline $baseline_count — lower $BASELINE_FILE to lock in the improvement"
fi
log_ok "$STEP" \
  "clones $current_count/$baseline_count, lines ${lines_pct}% ≤ ${JSCPD_PCT_MAX}% (jscpd $JSCPD_VERSION)"
