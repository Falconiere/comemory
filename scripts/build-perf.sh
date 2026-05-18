#!/usr/bin/env bash
# Measure cold / warm / release build wall-clock and write a summary.
# Output: target/build-perf/{cargo-timings-*.html,summary.json}
# Optionally appends a markdown row to docs/build-perf.md when --append-md is set.

set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=scripts/lib/common.sh
source "$HERE/lib/common.sh"
# shellcheck source=scripts/lib/perf.sh
source "$HERE/lib/perf.sh"

STEP="build-perf"
APPEND_MD=0
WARM_RUNS=5
[[ "${1:-}" == "--append-md" ]] && APPEND_MD=1

command -v jq >/dev/null 2>&1 || die "$STEP" "jq is required (brew install jq)"

OUT_DIR="$PROJECT_ROOT/target/build-perf"
mkdir -p "$OUT_DIR"

log_info "$STEP" "rustc $(rustc --version) / cargo $(cargo --version)"

log_info "$STEP" "cold build (cargo clean && cargo build --timings)"
(cd "$PROJECT_ROOT" && cargo clean >/dev/null)
COLD_S="$(perf_time_once cold cargo build --manifest-path "$PROJECT_ROOT/Cargo.toml" \
  --timings)"
cp "$PROJECT_ROOT/target/cargo-timings/cargo-timing.html" \
   "$OUT_DIR/cargo-timings-cold.html"
COLD_HTML="$PROJECT_ROOT/target/cargo-timings/cargo-timing.html"

log_info "$STEP" "warm incremental builds (touch src/lib.rs, ${WARM_RUNS} runs)"
touch "$PROJECT_ROOT/src/lib.rs"
read -r WARM_P50 WARM_P95 < <(perf_time_runs warm "$WARM_RUNS" \
  bash -c "touch '$PROJECT_ROOT/src/lib.rs' && cargo build --manifest-path '$PROJECT_ROOT/Cargo.toml'")

log_info "$STEP" "release build (cargo clean && cargo build --release --timings)"
(cd "$PROJECT_ROOT" && cargo clean >/dev/null)
RELEASE_S="$(perf_time_once release cargo build --manifest-path "$PROJECT_ROOT/Cargo.toml" \
  --release --timings)"
cp "$PROJECT_ROOT/target/cargo-timings/cargo-timing.html" \
   "$OUT_DIR/cargo-timings-release.html"

TOP10="$(perf_top_crates "$COLD_HTML" 10)"
SCCACHE_STATE="absent"
[[ -n "${RUSTC_WRAPPER:-}" ]] && SCCACHE_STATE="${RUSTC_WRAPPER}"

COMMIT="$(git -C "$PROJECT_ROOT" rev-parse --short HEAD)"
DATE="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

jq -n \
  --arg commit "$COMMIT" \
  --arg date "$DATE" \
  --arg rustc "$(rustc --version)" \
  --arg cargo "$(cargo --version)" \
  --arg sccache "$SCCACHE_STATE" \
  --argjson cold "$COLD_S" \
  --argjson warm_p50 "$WARM_P50" \
  --argjson warm_p95 "$WARM_P95" \
  --argjson release "$RELEASE_S" \
  --argjson top10 "$TOP10" \
  '{ commit:$commit, date:$date, rustc:$rustc, cargo:$cargo, sccache:$sccache,
     cold_s:$cold, warm_p50_s:$warm_p50, warm_p95_s:$warm_p95,
     release_s:$release, top10_crates:$top10 }' \
  > "$OUT_DIR/summary.json"

log_ok "$STEP" "cold=${COLD_S}s warm_p50=${WARM_P50}s warm_p95=${WARM_P95}s release=${RELEASE_S}s"
log_info "$STEP" "summary: $OUT_DIR/summary.json"

if [[ "$APPEND_MD" -eq 1 ]]; then
  MD="$PROJECT_ROOT/docs/build-perf.md"
  NOTES="${BUILD_PERF_NOTES:-}"
  NOTES="${NOTES//|/\\|}"
  printf "| %s | %s | %s | %s | %s | %s | %s | %s |\n" \
    "$DATE" "$COMMIT" "$COLD_S" "$WARM_P50" "$WARM_P95" "$RELEASE_S" "$SCCACHE_STATE" "$NOTES" \
    >> "$MD"
  log_ok "$STEP" "appended row to $MD"
fi
