#!/usr/bin/env bash
# AST duplicate ratchet over production Rust, using a PINNED cargo-dupes
# build (https://github.com/mpecan/cargo-dupes). Complements
# scripts/dup-check.sh (similarity-rs whole-function APTED) and
# scripts/jscpd-check.sh (token clones): cargo-dupes normalizes syn ASTs for
# exact + near-duplicate groups.
#
# WHY THE VERSION IS PINNED. Group counts move between releases. An unpinned
# binary would make the baseline files a measurement of the runner. Mismatch
# is a HARD FAILURE everywhere.
#
# .github/workflows/test.yml DERIVES the version it installs from the exact
# assignment line below. Keep it on one line, spelled exactly as it is.
#
# NOT in scripts/check-all.sh's GATES — needs a separately installed tool;
# grouped with the other tool-pinned gates in the justfile `qa` recipe and CI.
set -euo pipefail

CARGO_DUPES_VERSION="0.2.1"
# Match scripts/dup-check.sh's similarity threshold so near-dup semantics stay
# comparable across the two function-oriented detectors.
NEAR_THRESHOLD="0.85"
# Hard ceiling on near-duplicate *lines* percentage. Same 5% residual policy
# as scripts/dup-check.sh / scripts/jscpd-check.sh. Exact-duplicate % is still
# above 5% on this tree — it ratchets by group count only until burned under
# the ceiling (then enable the exact % check the same way).
CARGO_DUPES_NEAR_PCT_MAX=5

HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"
cd "$PROJECT_ROOT"

STEP="cargo-dupes-check"
INSTALL_HINT="cargo install cargo-dupes --version $CARGO_DUPES_VERSION --locked"
EXACT_BASELINE_FILE="$PROJECT_ROOT/cargo-dupes-exact-baseline.txt"
NEAR_BASELINE_FILE="$PROJECT_ROOT/cargo-dupes-near-baseline.txt"

DUPES_BIN="$(command -v cargo-dupes 2>/dev/null || true)"
if [[ -z "$DUPES_BIN" ]]; then
  if [[ -n "${CI:-}" ]]; then
    log_err "$STEP" \
      "cargo-dupes is required in CI but is not on PATH — install it with: $INSTALL_HINT"
    exit 1
  fi
  printf '%s[%s]%s %s[warn]%s the cargo-dupes gate DID NOT RUN — cargo-dupes is not installed\n' \
    "$C_DIM" "$STEP" "$C_RST" "$C_YLW" "$C_RST" >&2
  printf '%s[%s]%s %s[warn]%s install the pinned build to run it: %s\n' \
    "$C_DIM" "$STEP" "$C_RST" "$C_YLW" "$C_RST" "$INSTALL_HINT" >&2
  exit 0
fi

version_output="$("$DUPES_BIN" -V 2>/dev/null || true)"
installed_version="$(printf '%s\n' "$version_output" | awk 'NF { print $NF; exit }')"
if [[ -z "$installed_version" ]]; then
  log_err "$STEP" \
    "could not read the installed cargo-dupes version (pinned to $CARGO_DUPES_VERSION); install with: $INSTALL_HINT"
  exit 1
fi
if [[ "$installed_version" != "$CARGO_DUPES_VERSION" ]]; then
  log_err "$STEP" \
    "cargo-dupes $installed_version is on PATH but this gate is pinned to $CARGO_DUPES_VERSION — counts from another build are not comparable; install with: $INSTALL_HINT"
  exit 1
fi

for baseline_file in "$EXACT_BASELINE_FILE" "$NEAR_BASELINE_FILE"; do
  if [[ ! -f "$baseline_file" ]]; then
    log_err "$STEP" "missing $baseline_file (baseline ratchet file)"
    exit 1
  fi
done
exact_baseline="$(tr -d '[:space:]' < "$EXACT_BASELINE_FILE")"
near_baseline="$(tr -d '[:space:]' < "$NEAR_BASELINE_FILE")"
if ! [[ "$exact_baseline" =~ ^[0-9]+$ && "$near_baseline" =~ ^[0-9]+$ ]]; then
  log_err "$STEP" \
    "baseline files must each contain a single integer group count"
  exit 1
fi

stats_json="$(mktemp -t comemory-cargo-dupes.XXXXXX)"
trap 'rm -f "$stats_json"' EXIT

# stats JSON is the measurement; check enforces the ceilings.
set +e
"$DUPES_BIN" stats \
  --path src \
  --exclude-tests \
  --exclude '**/tests/**' \
  --threshold "$NEAR_THRESHOLD" \
  --format json \
  >"$stats_json" 2>"${stats_json}.err"
stats_ec=$?
set -e
if (( stats_ec != 0 )); then
  log_err "$STEP" "cargo-dupes stats exited $stats_ec; see ${stats_json}.err"
  trap - EXIT
  exit 1
fi

parsed="$(python3 -c '
import json, sys
d = json.load(open(sys.argv[1]))
print("%s %s %s %.6f %.6f" % (
    d["exact_duplicate_groups"],
    d["near_duplicate_groups"],
    d["total_code_units"],
    float(d["exact_duplicate_percent"]),
    float(d["near_duplicate_percent"]),
))
' "$stats_json")"
exact_count="$(printf '%s\n' "$parsed" | awk '{ print $1 }')"
near_count="$(printf '%s\n' "$parsed" | awk '{ print $2 }')"
units="$(printf '%s\n' "$parsed" | awk '{ print $3 }')"
exact_pct="$(printf '%s\n' "$parsed" | awk '{ print $4 }')"
near_pct="$(printf '%s\n' "$parsed" | awk '{ print $5 }')"
if ! [[ "$exact_count" =~ ^[0-9]+$ && "$near_count" =~ ^[0-9]+$ && "$units" =~ ^[0-9]+$ ]]; then
  log_err "$STEP" "could not parse cargo-dupes stats JSON"
  trap - EXIT
  exit 1
fi

# ANTI-VACUOUS: an empty/failed scan would report 0 units.
if (( units == 0 )); then
  log_err "$STEP" "cargo-dupes reported 0 code units — scan was vacuous"
  trap - EXIT
  exit 1
fi

log_info "$STEP" \
  "exact groups $exact_count/$exact_baseline (${exact_pct}%), near groups $near_count/$near_baseline (${near_pct}%, ceiling ${CARGO_DUPES_NEAR_PCT_MAX}%) over $units units (cargo-dupes $CARGO_DUPES_VERSION, threshold $NEAR_THRESHOLD)"

failed=0
if awk -v p="$near_pct" -v max="$CARGO_DUPES_NEAR_PCT_MAX" 'BEGIN { exit !(p > max) }'; then
  log_err "$STEP" \
    "near-duplicate lines percentage $near_pct exceeds the ${CARGO_DUPES_NEAR_PCT_MAX}% ceiling"
  failed=1
fi
if (( exact_count > exact_baseline )); then
  log_err "$STEP" \
    "exact duplicate groups $exact_count exceed baseline $exact_baseline"
  failed=1
fi
if (( near_count > near_baseline )); then
  log_err "$STEP" \
    "near duplicate groups $near_count exceed baseline $near_baseline"
  failed=1
fi
if (( failed != 0 )); then
  trap - EXIT
  exit 1
fi
if (( exact_count < exact_baseline )); then
  log_info "$STEP" \
    "exact groups $exact_count below baseline $exact_baseline — lower $EXACT_BASELINE_FILE to lock in the improvement"
fi
if (( near_count < near_baseline )); then
  log_info "$STEP" \
    "near groups $near_count below baseline $near_baseline — lower $NEAR_BASELINE_FILE to lock in the improvement"
fi
# Exact % is still above the 5% project ceiling on this tree; surface it so
# burns stay visible without failing the gate until the count ratchet has
# brought exact under CARGO_DUPES_NEAR_PCT_MAX.
if awk -v p="$exact_pct" -v max="$CARGO_DUPES_NEAR_PCT_MAX" 'BEGIN { exit !(p > max) }'; then
  log_info "$STEP" \
    "exact-duplicate lines ${exact_pct}% still above the ${CARGO_DUPES_NEAR_PCT_MAX}% project ceiling — burn exact groups; count ratchet only until then"
fi
log_ok "$STEP" \
  "exact $exact_count/$exact_baseline (${exact_pct}%), near $near_count/$near_baseline (${near_pct}% ≤ ${CARGO_DUPES_NEAR_PCT_MAX}%) (cargo-dupes $CARGO_DUPES_VERSION)"
