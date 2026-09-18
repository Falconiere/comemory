#!/usr/bin/env bash
# Unused-dependency gate over this crate's manifest, using a PINNED
# cargo-machete build.
#
# WHERE THIS RUNS (issue #205). Until #205 this script had no header at all and
# appeared in no file under .github/workflows/, so it only ever ran when
# somebody typed `just qa` — and when cargo-machete was absent it logged a dim
# info line and exited 0, in CI as much as locally. It now runs in BOTH the
# justfile's `qa` recipe and its own `Run machete gate` step in
# .github/workflows/test.yml, and a missing tool fails CI.
#
# WHY THE VERSION IS PINNED. cargo-machete's detection is textual and imprecise
# by design, so which dependencies it flags changes between releases. A
# different version is a HARD FAILURE in every environment, local included: a
# green `just qa` has to mean the same thing as a green CI run. (#202 made the
# same call for similarity-rs, #205 for cargo-deny.)
#
# .github/workflows/test.yml DERIVES the version it installs from the exact
# assignment line below, so the workflow and this gate cannot drift apart.
# Keep it on one line, spelled exactly as it is.
#
# NOT in scripts/check-all.sh's GATES, deliberately — see the header comment
# there.
#
# ANTI-VACUOUS: THIS TOOL REPORTS SUCCESS WHEN IT ANALYSED NOTHING. Measured:
# run in a directory containing no Cargo.toml at all, cargo-machete prints
#
#   Analyzing dependencies of crates in this directory...
#   cargo-machete didn't find any unused dependencies in this directory. Good job!
#
# and exits 0. Adding --with-metadata does not change it. The success output is
# byte-identical whether it parsed this repo's manifest or nothing whatsoever,
# and unlike similarity-rs it never reports how much it ingested — so there is
# no count to assert and #202's file-count guard is not available here.
#
# What this gate does instead is run a CANARY first: a copy of this repo's own
# Cargo.toml in a temp directory next to an empty src/lib.rs, scanned by the
# same pinned binary, which must exit 1 and name unused dependencies. That
# proves, on this run and this machine, that the binary parses THIS manifest,
# extracts its dependencies, scans sources and honours
# [package.metadata.cargo-machete]. If the canary does not trip, the real scan's
# "Good job!" is worth nothing and the gate fails rather than reporting a pass.
#
# Using the real manifest rather than an invented crate is deliberate:
# cargo-machete honours .gitignore/.ignore when searching for files, so an
# ignored Cargo.toml or src/ would still produce "Good job!" and exit 0 while an
# invented canary sailed through.
set -euo pipefail

CARGO_MACHETE_VERSION="0.9.2"

HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"
cd "$PROJECT_ROOT"

STEP="machete-check"
INSTALL_HINT="cargo install cargo-machete --version $CARGO_MACHETE_VERSION --locked"

# Resolved ONCE, then invoked by absolute path everywhere below, so the binary
# whose version was approved is provably the binary that runs.
MACHETE_BIN="$(command -v cargo-machete 2>/dev/null || true)"
if [[ -z "$MACHETE_BIN" ]]; then
  if [[ -n "${CI:-}" ]]; then
    log_err "$STEP" \
      "cargo-machete is required in CI but is not on PATH — install it with: $INSTALL_HINT"
    exit 1
  fi
  # Loud on purpose. This gate previously logged a dim info line here, which is
  # indistinguishable from a pass at a glance.
  printf '%s[%s]%s %s[warn]%s the unused-dependency gate DID NOT RUN — cargo-machete is not installed\n' \
    "$C_DIM" "$STEP" "$C_RST" "$C_YLW" "$C_RST" >&2
  printf '%s[%s]%s %s[warn]%s install the pinned build to run it: %s\n' \
    "$C_DIM" "$STEP" "$C_RST" "$C_YLW" "$C_RST" "$INSTALL_HINT" >&2
  exit 0
fi

# `|| true` scoped to the tool call alone — see the same note in deny-check.sh.
# cargo-machete prints a BARE version number ("0.9.2"), where cargo-deny prints
# "cargo-deny 0.20.2"; taking the last field of the first non-empty line handles
# both shapes.
version_output="$("$MACHETE_BIN" --version 2>/dev/null || true)"
installed_version="$(printf '%s\n' "$version_output" | awk 'NF { print $NF; exit }')"
if [[ -z "$installed_version" ]]; then
  log_err "$STEP" \
    "could not read the installed cargo-machete version (this gate is pinned to $CARGO_MACHETE_VERSION); install the pinned build with: $INSTALL_HINT"
  exit 1
fi
if [[ "$installed_version" != "$CARGO_MACHETE_VERSION" ]]; then
  log_err "$STEP" \
    "cargo-machete $installed_version is on PATH but this gate is pinned to $CARGO_MACHETE_VERSION — a verdict from another build is not the verdict CI will reach; install the pinned build with: $INSTALL_HINT"
  exit 1
fi

# ONE flag list, used by BOTH the canary and the real scan. They are empty
# today, but the canary only proves the real scan is live if the two runs use
# the same detection rules — add --with-metadata to one and not the other and
# the proof quietly stops covering what it claims to. Sharing the array makes
# divergence impossible rather than merely discouraged.
MACHETE_FLAGS=()

MANIFEST="$PROJECT_ROOT/Cargo.toml"
if [[ ! -f "$MANIFEST" ]]; then
  log_err "$STEP" "missing $MANIFEST — there is nothing for this gate to scan"
  exit 1
fi

# ---------------------------------------------------------------------------
# Canary: prove the binary detects, on a copy of the real manifest.
# ---------------------------------------------------------------------------
canary_dir="$(mktemp -d -t comemory-machete-canary.XXXXXX)"
trap 'rm -rf "$canary_dir"' EXIT
# The canary must live OUTSIDE the tree the real scan covers. If the temporary
# directory were ever inside $PROJECT_ROOT — GNU mktemp honours $TMPDIR, and
# some CI runners point it under the workspace — the real scan would walk into
# the canary and report its dozens of deliberately-unused dependencies as
# genuine findings, failing the gate for a reason that has nothing to do with
# the tree. Checked rather than assumed, and both sides resolved with pwd -P so
# a symlinked temp directory cannot slip past the comparison.
# (Note for anyone testing this on a Mac: BSD mktemp -t IGNORES $TMPDIR and
# always uses the Darwin per-user temp dir, so setting $TMPDIR will not
# exercise this branch there — stub `mktemp` on PATH instead.)
canary_real="$(cd "$canary_dir" && pwd -P)"
root_real="$(cd "$PROJECT_ROOT" && pwd -P)"
if [[ "$canary_real" == "$root_real"/* ]]; then
  log_err "$STEP" \
    "the canary directory $canary_real is inside the repository at $root_real, so the real scan would walk into it and report its intentionally-unused dependencies as findings — point the temporary directory outside the repository"
  exit 1
fi
mkdir -p "$canary_dir/src"
cp "$MANIFEST" "$canary_dir/Cargo.toml"
# No dependency is used from here, so every non-ignored one must be reported.
# cargo-machete parses the manifest and scans sources textually — it never runs
# cargo — so the copied manifest's references to src/main.rs, benches/ and
# build.rs do not need to exist.
printf '// Intentionally empty: the canary uses none of the declared dependencies.\n' \
  > "$canary_dir/src/lib.rs"

set +e
canary_output="$("$MACHETE_BIN" ${MACHETE_FLAGS[@]+"${MACHETE_FLAGS[@]}"} "$canary_dir" 2>&1)"
canary_rc=$?
set -e
if (( canary_rc == 0 )); then
  log_err "$STEP" \
    "the cargo-machete canary DID NOT TRIP: handed a copy of $MANIFEST beside an empty src/lib.rs, where every dependency is unused, it exited 0. This binary detects nothing, so a clean scan of the real tree would prove nothing. Output was: $canary_output"
  exit 1
fi
canary_found="$(printf '%s\n' "$canary_output" \
  | awk '/^\t/ { n++ } END { print n + 0 }')"
if (( canary_found == 0 )); then
  log_err "$STEP" \
    "the cargo-machete canary exited $canary_rc but named no unused dependency, so its failure was not a detection. Output was: $canary_output"
  exit 1
fi

# ---------------------------------------------------------------------------
# The real scan.
# ---------------------------------------------------------------------------
# The path is passed EXPLICITLY rather than inherited from the working
# directory, and cargo-machete echoes back the path it was handed, so the banner
# below proves the scan was aimed where this gate intended.
set +e
scan_output="$("$MACHETE_BIN" ${MACHETE_FLAGS[@]+"${MACHETE_FLAGS[@]}"} "$PROJECT_ROOT" 2>&1)"
scan_rc=$?
set -e

if ! printf '%s\n' "$scan_output" | grep -qF "Analyzing dependencies of crates in $PROJECT_ROOT"; then
  log_err "$STEP" \
    "cargo-machete did not report analysing $PROJECT_ROOT — it was aimed somewhere else. Output was: $scan_output"
  exit 1
fi

if (( scan_rc != 0 )); then
  printf '%s\n' "$scan_output" >&2
  log_err "$STEP" "cargo-machete found unused dependencies (exit $scan_rc) — see the list above"
  exit "$scan_rc"
fi

log_ok "$STEP" \
  "cargo-machete $CARGO_MACHETE_VERSION: no unused dependencies in $PROJECT_ROOT (canary over a copy of Cargo.toml flagged $canary_found, so the detector is live)"
