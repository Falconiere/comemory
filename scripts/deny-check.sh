#!/usr/bin/env bash
# Licence + advisory policy gate over the whole crate graph, using a PINNED
# cargo-deny build.
#
# WHERE THIS RUNS (issue #205). Until #205 this header claimed it was "invoked
# from `just qa` and CI", and the CI half was false: the script appeared in no
# file under .github/workflows/, so `cargo deny check` only ever ran when
# somebody remembered to type `just qa`. It now runs in BOTH: the `qa` recipe in
# the justfile, and its own `Run deny gate` step in .github/workflows/test.yml.
#
# WHY THE VERSION IS PINNED. cargo-deny's licence expression evaluation, its
# deny.toml schema and its advisory handling all move between releases, so an
# unpinned tool reports on whichever build happened to be on PATH rather than on
# the policy this repo wrote down. A different version is a HARD FAILURE in
# every environment, local included: the point of this gate is that a green
# `just qa` means the same thing as a green CI run, and a verdict from another
# build does not deliver that. (#202 made the same call for similarity-rs.)
#
# .github/workflows/test.yml DERIVES the version it installs from the exact
# assignment line below, so the workflow and this gate cannot drift apart.
# Keep it on one line, spelled exactly as it is.
#
# NOT in scripts/check-all.sh's GATES, deliberately — see the header comment
# there. It needs a separately installed tool, which in check-all would mean
# either hard-failing every contributor who lacks it or skipping silently.
#
# ANTI-VACUOUS. #200 found that similarity-rs silently discarded 53 of the files
# dup-check.sh handed it, so that gate had been passing over files nobody
# analysed. cargo-deny does not have that failure mode — handed a directory with
# no Cargo.toml it exits 1, and with no deny.toml it exits 4 because its default
# allow-list is empty — but it can still be made to check LESS while exiting 0:
# a narrowed invocation runs fewer checks, and a [graph] targets/exclude entry
# shrinks the graph. Exit 0 alone is therefore not accepted as proof. Five
# things about the run are asserted, all parsed from cargo-deny's own output:
#
#   1. it read THIS repo's deny.toml, not built-in defaults;
#   2. it gathered a plausible number of crates, floored against Cargo.lock;
#   3. it fetched the RustSec advisory database rather than running on nothing;
#   4. all four checks reported, each `ok` — `cargo deny check licenses` also
#      exits 0 while advisories go unchecked;
#   5. the licence check made a non-zero number of determinations.
set -euo pipefail

CARGO_DENY_VERSION="0.20.2"

HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"
cd "$PROJECT_ROOT"

STEP="deny-check"
INSTALL_HINT="cargo install cargo-deny --version $CARGO_DENY_VERSION --locked"

# Resolved ONCE, then invoked by absolute path everywhere below. The previous
# version of this script tested `command -v cargo-deny` and then ran `cargo deny
# check`, so the binary whose version was approved was not provably the binary
# that ran: a cargo subcommand shim, or a second copy earlier on PATH, would
# separate them silently.
DENY_BIN="$(command -v cargo-deny 2>/dev/null || true)"
if [[ -z "$DENY_BIN" ]]; then
  if [[ -n "${CI:-}" ]]; then
    log_err "$STEP" \
      "cargo-deny is required in CI but is not on PATH — install it with: $INSTALL_HINT"
    exit 1
  fi
  # Loud on purpose: a dim info line is indistinguishable from a pass, which is
  # how these gates went unnoticed in the first place.
  printf '%s[%s]%s %s[warn]%s the licence/advisory gate DID NOT RUN — cargo-deny is not installed\n' \
    "$C_DIM" "$STEP" "$C_RST" "$C_YLW" "$C_RST" >&2
  printf '%s[%s]%s %s[warn]%s install the pinned build to run it: %s\n' \
    "$C_DIM" "$STEP" "$C_RST" "$C_YLW" "$C_RST" "$INSTALL_HINT" >&2
  exit 0
fi

# The `|| true` is scoped to the TOOL call alone, not to the whole pipeline.
# Under set -e a binary that exists but exits non-zero (broken install, wrong
# arch, missing dylib) would otherwise kill the script here, making the
# empty-version branch below — the one carrying $INSTALL_HINT — dead code in
# exactly the case it was written for.
version_output="$("$DENY_BIN" --version 2>/dev/null || true)"
installed_version="$(printf '%s\n' "$version_output" | awk 'NF { print $NF; exit }')"
if [[ -z "$installed_version" ]]; then
  log_err "$STEP" \
    "could not read the installed cargo-deny version (this gate is pinned to $CARGO_DENY_VERSION); install the pinned build with: $INSTALL_HINT"
  exit 1
fi
if [[ "$installed_version" != "$CARGO_DENY_VERSION" ]]; then
  log_err "$STEP" \
    "cargo-deny $installed_version is on PATH but this gate is pinned to $CARGO_DENY_VERSION — a verdict from another build is not the verdict CI will reach; install the pinned build with: $INSTALL_HINT"
  exit 1
fi

CONFIG_FILE="$PROJECT_ROOT/deny.toml"
if [[ ! -f "$CONFIG_FILE" ]]; then
  log_err "$STEP" "missing $CONFIG_FILE — without it cargo-deny would fall back to its own defaults"
  exit 1
fi

# The crate-count floor is RELATIVE to Cargo.lock, so it never goes stale as
# dependencies churn. Measured on this tree: cargo-deny gathered 353 crates
# against 375 [[package]] entries (94%), the gap being the target filtering it
# legitimately does. Half the lock therefore leaves wide headroom while still
# catching a collapse — a wrong manifest, or a [graph] targets/exclude entry
# that quietly shrinks what gets checked.
LOCK_FILE="$PROJECT_ROOT/Cargo.lock"
if [[ ! -f "$LOCK_FILE" ]]; then
  log_err "$STEP" "missing $LOCK_FILE — the crate-count floor cannot be computed without it"
  exit 1
fi
# `|| true` because grep -c exits 1 when the count is legitimately 0, which
# under set -e would kill the script before the explicit zero check below can
# give a better message. The zero case is NOT swallowed: it is caught on the
# next line, so the floor can never be computed from an empty or truncated lock.
locked_packages="$(grep -c '^\[\[package\]\]' "$LOCK_FILE" || true)"
if ! [[ "$locked_packages" =~ ^[0-9]+$ ]]; then
  log_err "$STEP" "could not read [[package]] entries from $LOCK_FILE (is it readable?)"
  exit 1
fi
if (( locked_packages == 0 )); then
  log_err "$STEP" \
    "$LOCK_FILE contains no [[package]] entries — a truncated or malformed lock would make the crate-count floor 0, which would make the floor check below vacuous"
  exit 1
fi
min_crates=$(( locked_packages / 2 ))

scan_out="$(mktemp -t comemory-deny-out.XXXXXX)"
scan_err="$(mktemp -t comemory-deny-err.XXXXXX)"
# Removed on every exit path EXCEPT the ones that print a location for a human
# to read; each of those disarms the trap first.
trap 'rm -f "$scan_out" "$scan_err"' EXIT

# --log-level info is what makes this gate checkable: at the default level
# cargo-deny prints a single `advisories ok, bans ok, ...` summary line and
# nothing else, while at info it prints a per-check block on stdout (with error,
# warning and note counts) and the provenance lines on stderr.
# --color never keeps ANSI escapes out of the parsed text; CARGO_TERM_COLOR is
# an env var cargo-deny honours, so "not a TTY" is not sufficient on its own.
set +e
"$DENY_BIN" --log-level info --color never check > "$scan_out" 2> "$scan_err"
deny_rc=$?
set -e

if (( deny_rc != 0 )); then
  cat "$scan_err" >&2
  cat "$scan_out" >&2
  log_err "$STEP" "cargo deny check failed (exit $deny_rc) — see the output above"
  exit "$deny_rc"
fi

keep_logs() {
  trap - EXIT
  log_err "$STEP" "full output kept at $scan_out (stdout) and $scan_err (stderr)"
}

# (1) This repo's policy, not cargo-deny's defaults or a parent directory's.
if ! grep -qF "using config from $CONFIG_FILE" "$scan_err"; then
  log_err "$STEP" "cargo-deny did not report reading $CONFIG_FILE — it may have fallen back to built-in defaults"
  keep_logs
  exit 1
fi

# (2) A real graph, floored against the lock.
# Captured as a non-space token, NOT as digits: a digit-only capture makes a
# garbled value ("gathered lots crates") fall through to the "did not report"
# branch below, leaving the non-numeric branch dead code in exactly the case it
# was written for — the shape #202's review found in dup-check.sh's version
# probe. Capturing the token and validating it separately keeps both messages
# accurate and both branches reachable.
gathered="$(sed -n 's/.*gathered \([^ ][^ ]*\) crates.*/\1/p' "$scan_err" | head -1)"
if [[ -z "$gathered" ]]; then
  log_err "$STEP" "cargo-deny did not report how many crates it gathered"
  keep_logs
  exit 1
fi
if ! [[ "$gathered" =~ ^[0-9]+$ ]]; then
  # A non-numeric value evaluates as 0 inside (( )), which would compare as
  # "below the floor" and fail — the safe direction — but say so plainly.
  log_err "$STEP" "cargo-deny reported a non-numeric crate count: '$gathered'"
  keep_logs
  exit 1
fi
if (( gathered < min_crates )); then
  log_err "$STEP" \
    "cargo-deny gathered only $gathered crates, below the floor of $min_crates (half the $locked_packages [[package]] entries in Cargo.lock) — the graph collapsed, so this run checked far less than the tree"
  keep_logs
  exit 1
fi

# (3) Advisories checked against a database that was actually fetched.
if ! grep -q 'advisory database .* fetched in' "$scan_err"; then
  log_err "$STEP" "cargo-deny did not report fetching the advisory database — the advisory check would be running on stale or absent data"
  keep_logs
  exit 1
fi

# (4) All four checks ran, and each said ok.
for check_name in advisories bans licenses sources; do
  if ! awk -v want="$check_name" '$1 == want && $2 == "ok:" { found = 1 } END { exit !found }' "$scan_out"; then
    log_err "$STEP" "cargo-deny did not report '$check_name' as ok — exit 0 alone does not prove every check ran"
    keep_logs
    exit 1
  fi
done

# (5) The licence check made real determinations. Its note count is one per
# crate licence resolved, so zero means it ran over nothing.
license_notes="$(awk '$1 == "licenses" && $2 == "ok:" { print $7; exit }' "$scan_out")"
if ! [[ "$license_notes" =~ ^[0-9]+$ ]] || (( license_notes == 0 )); then
  log_err "$STEP" "the licence check recorded ${license_notes:-no} determinations — it evaluated no crate licences at all"
  keep_logs
  exit 1
fi

# Four lines, and they carry the warning counts (multiple-versions duplicates
# are `warn` by deny.toml policy), so a clean run still shows what was found.
cat "$scan_out"
log_ok "$STEP" \
  "cargo-deny $CARGO_DENY_VERSION: all four checks ok over $gathered crates (floor $min_crates), $license_notes licence determinations, advisory database fetched"
