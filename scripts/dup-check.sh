#!/usr/bin/env bash
# Duplication ratchet over src/, using a PINNED similarity-rs build.
#
# RUST ONLY — a deliberately recorded limitation. similarity-rs parses Rust and
# nothing else: handed the 53 tracked scripts/*.sh paths it silently discarded
# them and reported "Checking 411 files", and run over those 53 alone it prints
# "No Rust files found in the specified paths." and exits 0 even at threshold
# 0.5. This script and docs/dup-debt.md both used to claim "src/ + scripts/"
# coverage, which was never true. Shell scripts are therefore NOT passed at all
# rather than passed and ignored: an inert path inflates the target count and
# would let the anti-vacuous guard below pass on files nobody analyzes.
# Duplication in scripts/ is currently unguarded; catching it needs a
# shell-capable detector, not a longer path list here.
#
# WHY THE VERSION IS PINNED (issue #200). similarity-rs scores with APTED
# tree-edit distance and its results move between releases, so an unpinned tool
# measures whichever build happened to be on PATH rather than the tree. That is
# not hypothetical: the previous baseline of 113 did not reproduce on its own
# tree — similarity-rs 0.5.0 run against commit 16286113, the commit that wrote
# that 113, reports 140 — so every comparison it fed was apples-to-oranges. The
# version below is therefore authoritative and a mismatch is a HARD FAILURE in
# every environment, local included: a count from another build cannot be
# compared to dup-baseline.txt at all, and failing loudly beats passing wrongly.
#
# .github/workflows/test.yml DERIVES the version it installs from the exact
# assignment line below, so the workflow and this gate cannot drift apart.
# Keep it on one line, spelled exactly as it is.
#
# Colocated tests under src/**/tests/ are excluded from the scan: CORE excludes
# tests from copy-paste detection ("repeated setup there is not the duplication
# worth chasing"). similarity-rs's own --exclude flag was measured to NOT filter
# its positional PATHS list (file count stayed unchanged with it set), so the
# file list is built explicitly instead, via git ls-files filtered to drop any
# path containing /tests/.
#
# RATCHET, not an allowlist: similarity-rs has no per-pair suppression flag
# (confirmed via `similarity-rs --help`), so the near-duplicate pairs documented
# in docs/dup-debt.md cannot be silenced pair-by-pair. Instead this gate
# compares the CURRENT duplicate-pair COUNT against the baseline recorded in
# dup-baseline.txt (repo root, tracked — same convention as coverage-floor.txt
# and store-leak-baseline.txt).
#
# ONE-SIDED, deliberately — a ceiling, not an exact match. Exceeding the
# baseline fails; coming in under it passes with a nudge to lower the file.
# scripts/store-chokepoint-check.sh is the repo's TWO-SIDED ratchet and its
# header draws this contrast on purpose: that gate is driving a counted debt to
# zero under an active burn-down, so it must fail in both directions to force
# each PR to record its reduction. Duplication has no zero target — the largest
# clusters here are documented deliberate per-language and per-table repetition
# — and no burn-down campaign, so failing a PR for refactoring unrelated code
# downward would be friction with nothing behind it.
#
# NOT in scripts/check-all.sh's GATES, deliberately. It needs a separately
# installed tool, which in check-all would mean either hard-failing every
# contributor who lacks it or skipping silently — and a guardrail reporting
# success while checking nothing is the failure mode this gate exists to avoid.
# It is grouped instead with the other two tool-dependent gates (deny-check,
# machete-check) in the justfile's `qa` recipe, and runs authoritatively in CI,
# where the pinned version is guaranteed.
set -euo pipefail

SIMILARITY_RS_VERSION="0.5.0"

HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"
cd "$PROJECT_ROOT"

STEP="dup-check"
INSTALL_HINT="cargo install similarity-rs --version $SIMILARITY_RS_VERSION --locked"

if ! command -v similarity-rs >/dev/null 2>&1; then
  if [[ -n "${CI:-}" ]]; then
    log_err "$STEP" \
      "similarity-rs is required in CI but is not on PATH — install it with: $INSTALL_HINT"
    exit 1
  fi
  # Loud on purpose. This gate previously logged a dim info line here, which is
  # indistinguishable from a pass at a glance.
  printf '%s[%s]%s %s[warn]%s the duplication gate DID NOT RUN — similarity-rs is not installed\n' \
    "$C_DIM" "$STEP" "$C_RST" "$C_YLW" "$C_RST" >&2
  printf '%s[%s]%s %s[warn]%s install the pinned build to run it: %s\n' \
    "$C_DIM" "$STEP" "$C_RST" "$C_YLW" "$C_RST" "$INSTALL_HINT" >&2
  exit 0
fi

# The `|| true` is scoped to the TOOL call alone, not to the whole pipeline.
# Under set -e + pipefail a binary that exists but exits nonzero (broken
# install, wrong arch, missing dylib) would otherwise kill the script here,
# making the empty-version branch below — the one carrying $INSTALL_HINT — dead
# code in exactly the case it was written for. Keeping awk outside that `||`
# means a genuine awk failure still propagates instead of being swallowed.
version_output="$(similarity-rs --version 2>/dev/null || true)"
installed_version="$(printf '%s\n' "$version_output" | awk 'NF { print $NF; exit }')"
if [[ -z "$installed_version" ]]; then
  log_err "$STEP" \
    "could not read the installed similarity-rs version (this gate is pinned to $SIMILARITY_RS_VERSION); install the pinned build with: $INSTALL_HINT"
  exit 1
fi
if [[ "$installed_version" != "$SIMILARITY_RS_VERSION" ]]; then
  log_err "$STEP" \
    "similarity-rs $installed_version is on PATH but this gate is pinned to $SIMILARITY_RS_VERSION — counts from another build are not comparable to dup-baseline.txt; install the pinned build with: $INSTALL_HINT"
  exit 1
fi

BASELINE_FILE="$PROJECT_ROOT/dup-baseline.txt"
if [[ ! -f "$BASELINE_FILE" ]]; then
  log_err "$STEP" "missing $BASELINE_FILE (baseline ratchet file)"
  exit 1
fi
baseline_count="$(tr -d '[:space:]' < "$BASELINE_FILE")"
if ! [[ "$baseline_count" =~ ^[0-9]+$ ]]; then
  log_err "$STEP" "$BASELINE_FILE must contain a single integer count"
  exit 1
fi

mapfile -t targets < <(
  git ls-files 'src/*.rs' | grep -v '/tests/' \
    | while IFS= read -r f; do [[ -f "$f" ]] && printf '%s\n' "$f"; done
)
if (( ${#targets[@]} == 0 )); then
  log_err "$STEP" "no files to scan — the target list is empty, which would make this gate vacuous"
  exit 1
fi

# --fail-on-duplicates makes similarity-rs exit 1 whenever ANY duplicate is
# found, which is expected while the documented baseline is non-zero — so the
# exit code alone can no longer be the pass/fail signal. Capture the output
# unconditionally and let the count comparison below decide.
set +e
scan_output="$(similarity-rs --threshold 0.85 --fail-on-duplicates "${targets[@]}" 2>&1)"
set -e
scan_log="$(mktemp -t comemory-dup.XXXXXX)"
# Removed on every exit path EXCEPT the ones that print its location for a human
# to read; each of those disarms the trap first. Without the trap, any early
# exit after this point leaks one temp file per run.
trap 'rm -f "$scan_log"' EXIT
printf '%s\n' "$scan_output" > "$scan_log"

# ANTI-VACUOUS: assert the tool actually ingested every file we handed it.
# similarity-rs announces "Checking N files for duplicates..." and silently
# drops anything it cannot parse. Without this check, a folder move, a glob
# typo, or a future file kind it stops understanding would shrink N, shrink the
# count, and sail through the deliberately one-sided ratchet green — printing a
# "lower the baseline" nudge while having scanned a fraction of the tree. That
# is the precise failure this gate exists to prevent, so it is asserted, not
# assumed.
# [0-9,_]+ then strip separators: the pinned build prints a bare integer, but a
# digit-grouped "1,234" would otherwise fail to parse and be reported as "did
# not report how many files it checked" — a confusing message for a tool that
# did report it. The integer guard below still validates the normalized value.
checked_count="$(printf '%s\n' "$scan_output" \
  | awk '/^Checking [0-9,_]+ file(s)? for duplicates/ { gsub(/[,_]/, "", $2); print $2; exit }')"
if [[ -z "$checked_count" ]]; then
  log_err "$STEP" \
    "similarity-rs did not report how many files it checked; see $scan_log"
  trap - EXIT   # keep the log: its path is named above
  exit 1
fi
if (( checked_count != ${#targets[@]} )); then
  log_err "$STEP" \
    "similarity-rs checked $checked_count files but ${#targets[@]} were handed to it — it silently skipped $(( ${#targets[@]} - checked_count )); see $scan_log"
  trap - EXIT   # keep the log: its path is named above
  exit 1
fi

if printf '%s\n' "$scan_output" | grep -q '^No duplicate functions found!$'; then
  current_count=0
else
  current_count="$(printf '%s\n' "$scan_output" \
    | awk -F': ' '/^Total duplicate pairs found:/ { print $2; exit }')"
  if [[ -z "$current_count" ]]; then
    log_err "$STEP" "could not parse similarity-rs output; see $scan_log"
    trap - EXIT   # keep the log: its path is named above
    exit 1
  fi
fi

# Validate the parsed count the same way the baseline is validated. Without
# this, a garbled or multi-line parse reaches (( current_count > baseline ))
# where bash evaluates a non-numeric string as 0 — which is FALSE, so the gate
# would report success having compared nothing. That is precisely the
# vacuous-green failure this gate exists to prevent, so it is checked, not
# assumed.
if ! [[ "$current_count" =~ ^[0-9]+$ ]]; then
  log_err "$STEP" \
    "parsed a non-numeric duplicate-pair count ('$current_count') from similarity-rs; see $scan_log"
  trap - EXIT   # keep the log: its path is named above
  exit 1
fi

if (( current_count > baseline_count )); then
  log_err "$STEP" \
    "near-duplicate count $current_count exceeds baseline $baseline_count (docs/dup-debt.md); see $scan_log"
  trap - EXIT   # keep the log: its path is named above
  exit 1
fi
if (( current_count < baseline_count )); then
  log_info "$STEP" \
    "near-duplicate count $current_count is below baseline $baseline_count — lower $BASELINE_FILE to lock in the improvement"
fi
log_ok "$STEP" "near-duplicate count $current_count within baseline $baseline_count (similarity-rs $SIMILARITY_RS_VERSION)"
