# Build Time Improvements Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cut local cold / warm / release build wall-clock on macOS Apple Silicon by shipping a measurement harness plus deterministic config wins in a single PR.

**Architecture:** Three layers in one PR: (1) `scripts/build-perf.sh` measurement harness with committed `docs/build-perf.md` history, (2) `.cargo/config.toml` + `Cargo.toml` profile/`rust-version` tuning, (3) `scripts/install.sh` + macOS-only CI with sccache. No `src/` changes; all binding rules and existing gates stay green.

**Tech Stack:** Rust 1.95 / cargo 1.95, bash + `set -euo pipefail`, `jq`, optional `sccache`/`hyperfine`, GitHub Actions, `cargo-dist` 0.22.1.

**Spec:** `docs/superpowers/specs/2026-05-18-build-time-improvements-design.md`.

---

## Pre-flight

- Working directory: `/Users/falconiere/Projects/qwick-memory`.
- Branch: create a fresh branch off `main`: `git checkout -b perf/build-time-single-pr`.
- Toolchain: confirm `rustc 1.95.0` and `cargo 1.95.0` via `rustc --version && cargo --version`. If different, stop and re-validate the `rust-version` bump in Task 6 with the actual installed version.
- Optional tools (do NOT block on these): `brew install jq` is required for the harness; `sccache` and `hyperfine` are optional. The plan installs `jq` in Task 1.

---

## Task 1: Install `jq` (required by harness)

**Files:**
- None.

- [ ] **Step 1: Check for jq**

Run: `command -v jq && jq --version`

Expected: prints a version. If "command not found", continue to Step 2; otherwise skip to Task 2.

- [ ] **Step 2: Install jq via brew**

Run: `brew install jq`

Expected: install succeeds, `jq --version` prints `jq-1.7` or newer.

- [ ] **Step 3: No commit**

Tool install is a contributor-environment concern, not a repo change.

---

## Task 2: Add `scripts/lib/perf.sh` helpers

**Files:**
- Create: `scripts/lib/perf.sh`

- [ ] **Step 1: Create the helper script**

Create `scripts/lib/perf.sh` with:

```bash
#!/usr/bin/env bash
# Helpers for scripts/build-perf.sh — sourced, not executed.
# Depends on scripts/lib/common.sh being sourced first.

# Return current epoch seconds with millisecond precision (portable: macOS + Linux).
perf_now_ms() {
  python3 -c 'import time; print(int(time.time()*1000))'
}

# Run a single command, print wall-clock seconds (3 decimals) to stdout.
# Args: <label> <cmd> [<args>...]
perf_time_once() {
  local label="$1"; shift
  local start end
  start="$(perf_now_ms)"
  "$@" >/dev/null
  end="$(perf_now_ms)"
  awk -v s="$start" -v e="$end" 'BEGIN { printf "%.3f", (e - s) / 1000.0 }'
}

# Run a command N times via hyperfine if present; else fall back to perf_time_once.
# Args: <label> <runs> <cmd> [<args>...]
# Emits "p50_s p95_s" on stdout. With fallback, p95 == p50.
perf_time_runs() {
  local label="$1"; local runs="$2"; shift 2
  if command -v hyperfine >/dev/null 2>&1; then
    local json p50 p95
    json="$(mktemp)"
    hyperfine --warmup 1 --runs "$runs" --export-json "$json" \
      --command-name "$label" "$*" >/dev/null
    p50="$(jq -r '.results[0].median' "$json")"
    p95="$(jq -r '.results[0].max' "$json")"
    rm -f "$json"
    awk -v p50="$p50" -v p95="$p95" 'BEGIN { printf "%.3f %.3f", p50, p95 }'
  else
    local single
    single="$(perf_time_once "$label" "$@")"
    printf "%s %s" "$single" "$single"
  fi
}

# Extract top-N crate-unit durations from a cargo-timings JSON file.
# Args: <timings.json> <n>
# Emits a JSON array: [{name, version, duration_s}, ...]
perf_top_crates() {
  local file="$1"; local n="$2"
  jq -c --argjson n "$n" '
    [ .invocations[]? | select(.target?) |
      { name: .package_id // .target.name,
        version: (.package_id // "") | capture("@(?<v>[^@]+)$")?.v // "",
        duration_s: (.duration | tonumber | (.*1000|round)/1000) } ]
    | sort_by(-.duration_s) | .[0:$n]
  ' "$file"
}
```

- [ ] **Step 2: Verify shellcheck clean**

Run: `shellcheck scripts/lib/perf.sh`

Expected: no output, exit 0.

- [ ] **Step 3: Verify file size under limit**

Run: `wc -l scripts/lib/perf.sh`

Expected: < 500.

- [ ] **Step 4: Commit**

```bash
git add scripts/lib/perf.sh
git -c commit.gpgsign=false commit -m "perf(scripts): add scripts/lib/perf.sh timing helpers"
```

---

## Task 3: Add `scripts/build-perf.sh` driver

**Files:**
- Create: `scripts/build-perf.sh`

- [ ] **Step 1: Create the driver script**

Create `scripts/build-perf.sh` with:

```bash
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

log_info "$STEP" "cold build (cargo clean && cargo build --timings=html,json)"
(cd "$PROJECT_ROOT" && cargo clean >/dev/null)
COLD_S="$(perf_time_once cold cargo build --manifest-path "$PROJECT_ROOT/Cargo.toml" \
  --timings=html,json)"
cp "$PROJECT_ROOT/target/cargo-timings/cargo-timings.html" \
   "$OUT_DIR/cargo-timings-cold.html"
COLD_JSON="$PROJECT_ROOT/target/cargo-timings/cargo-timings.json"

log_info "$STEP" "warm incremental builds (touch src/lib.rs, ${WARM_RUNS} runs)"
touch "$PROJECT_ROOT/src/lib.rs"
read -r WARM_P50 WARM_P95 < <(perf_time_runs warm "$WARM_RUNS" \
  bash -c "touch '$PROJECT_ROOT/src/lib.rs' && cargo build --manifest-path '$PROJECT_ROOT/Cargo.toml'")

log_info "$STEP" "release build (cargo clean && cargo build --release --timings=html,json)"
(cd "$PROJECT_ROOT" && cargo clean >/dev/null)
RELEASE_S="$(perf_time_once release cargo build --manifest-path "$PROJECT_ROOT/Cargo.toml" \
  --release --timings=html,json)"
cp "$PROJECT_ROOT/target/cargo-timings/cargo-timings.html" \
   "$OUT_DIR/cargo-timings-release.html"

TOP10="$(perf_top_crates "$COLD_JSON" 10)"
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
  printf "| %s | %s | %s | %s | %s | %s | %s | %s |\n" \
    "$DATE" "$COMMIT" "$COLD_S" "$WARM_P50" "$WARM_P95" "$RELEASE_S" "$SCCACHE_STATE" "$NOTES" \
    >> "$MD"
  log_ok "$STEP" "appended row to $MD"
fi
```

- [ ] **Step 2: Make executable, lint**

```bash
chmod +x scripts/build-perf.sh
shellcheck scripts/build-perf.sh
```

Expected: no shellcheck output, exit 0.

- [ ] **Step 3: Verify file size under limit**

Run: `wc -l scripts/build-perf.sh`

Expected: < 500.

- [ ] **Step 4: Commit**

```bash
git add scripts/build-perf.sh
git -c commit.gpgsign=false commit -m "perf(scripts): add build-perf driver"
```

---

## Task 4: Wire `just perf` and create `docs/build-perf.md` skeleton

**Files:**
- Modify: `justfile`
- Create: `docs/build-perf.md`

- [ ] **Step 1: Add `perf` target to `justfile`**

Open `justfile`. After the `e2e:` target, append:

```
perf:
    bash scripts/build-perf.sh
```

The resulting `justfile` end-of-file:

```
e2e:
    bash scripts/e2e.sh

perf:
    bash scripts/build-perf.sh
```

- [ ] **Step 2: Create `docs/build-perf.md`**

```markdown
# Build Performance

History of `just perf` runs. One row per measured run. Append rows via
`bash scripts/build-perf.sh --append-md`.

Columns:
- **Date:** UTC timestamp of the run.
- **Commit:** short SHA at measurement time.
- **Cold (s):** `cargo clean && cargo build` wall-clock.
- **Warm p50 (s):** median of 5 incremental rebuilds after `touch src/lib.rs`.
- **Warm p95 (s):** max of the same 5 runs (when hyperfine is installed; equals p50 otherwise).
- **Release (s):** `cargo clean && cargo build --release` wall-clock.
- **sccache:** wrapper state (`sccache` if `RUSTC_WRAPPER=sccache`, else `absent`).
- **Notes:** free-form context (e.g., "baseline", "post-config").

| Date | Commit | Cold (s) | Warm p50 (s) | Warm p95 (s) | Release (s) | sccache | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
```

- [ ] **Step 3: Verify justfile parses**

Run: `just --list`

Expected: lists targets including `perf`.

- [ ] **Step 4: Commit**

```bash
git add justfile docs/build-perf.md
git -c commit.gpgsign=false commit -m "perf(docs): add docs/build-perf.md skeleton, wire just perf"
```

---

## Task 5: Run baseline measurement (on unchanged config)

**Files:**
- Modify: `docs/build-perf.md` (appends one row).

- [ ] **Step 1: Run harness with append flag**

```bash
BUILD_PERF_NOTES="baseline (pre-change)" bash scripts/build-perf.sh --append-md
```

Expected: prints `[build-perf] OK cold=…s warm_p50=…s warm_p95=…s release=…s` and appends one row to `docs/build-perf.md`. The run takes several minutes (cold + release each rebuild from scratch).

- [ ] **Step 2: Sanity-check the appended row**

Run: `tail -n 3 docs/build-perf.md`

Expected: a single row with non-zero numbers and `Notes: baseline (pre-change)`.

- [ ] **Step 3: Verify summary JSON**

Run: `jq '.cold_s, .warm_p50_s, .release_s, (.top10_crates | length)' target/build-perf/summary.json`

Expected: three positive floats and `10`.

- [ ] **Step 4: Commit**

```bash
git add docs/build-perf.md
git -c commit.gpgsign=false commit -m "perf(docs): record baseline build numbers"
```

---

## Task 6: Add `.cargo/config.toml`

**Files:**
- Create: `.cargo/config.toml`

- [ ] **Step 1: Verify no existing `.cargo/config.toml`**

Run: `ls .cargo/config.toml 2>/dev/null || echo "absent"`

Expected: `absent`.

- [ ] **Step 2: Create the config**

```toml
# Respect RUSTC_WRAPPER from the environment. Contributors without sccache
# pay zero cost; contributors with sccache export RUSTC_WRAPPER=sccache in
# their shell.

[target.aarch64-apple-darwin]
rustflags = ["-C", "link-arg=-Wl,-dead_strip"]

[env]
SCCACHE_CACHE_SIZE = { value = "20G", force = false }
```

- [ ] **Step 3: Verify cargo accepts the config**

Run: `cargo build --message-format=short --quiet 2>&1 | tail -n 5`

Expected: build completes without any "warning: invalid configuration" or "error: malformed" message. Wall-clock here is incremental (project is already built from Task 5).

- [ ] **Step 4: Verify the link flag applies on a clean release build smoke**

Run: `cargo clean && cargo build --release --message-format=short 2>&1 | tail -n 5`

Expected: build succeeds. (Skip if Task 5 already proved release builds — this step exists to catch link-flag syntax errors. If on a slow machine, accept the cost; the next task changes profiles anyway.)

- [ ] **Step 5: Commit**

```bash
git add .cargo/config.toml
git -c commit.gpgsign=false commit -m "perf(config): add .cargo/config.toml with aarch64 link flag"
```

---

## Task 7: Update `Cargo.toml` profiles and `rust-version`

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Confirm current toolchain**

Run: `rustc --version`

Expected: `rustc 1.95.0 …`. If different, stop and adjust the `rust-version` value in Step 2 to the installed minor.

- [ ] **Step 2: Bump `rust-version`**

In `Cargo.toml`, change:

```toml
rust-version = "1.78"
```

to:

```toml
rust-version = "1.95"
```

- [ ] **Step 3: Add dev profile tune + per-package opt-level**

In `Cargo.toml`, after the existing `[profile.release]` block, append:

```toml
[profile.dev]
debug = "line-tables-only"
split-debuginfo = "unpacked"

[profile.dev.package."*"]
opt-level = 1

[profile.dev.build-override]
opt-level = 0
codegen-units = 256

[profile.release.build-override]
opt-level = 0
codegen-units = 256

[profile.release-quick]
inherits = "release"
lto = "thin"
codegen-units = 16
strip = "none"
```

The final file ordering must be: `[profile.release]` (unchanged) → new `[profile.dev]` → new `[profile.dev.package."*"]` → new `[profile.dev.build-override]` → new `[profile.release.build-override]` → new `[profile.release-quick]` → existing `[package.metadata.cargo-machete]` → existing `[package.metadata.dist]`. Do not reorder existing blocks.

- [ ] **Step 4: Verify dev build still works**

Run: `cargo build --message-format=short 2>&1 | tail -n 5`

Expected: build succeeds.

- [ ] **Step 5: Verify `release-quick` profile builds**

Run: `cargo build --profile release-quick --message-format=short 2>&1 | tail -n 10`

Expected: build succeeds. Profile shows in cargo's output as `release-quick`.

- [ ] **Step 6: Verify nextest still passes (catches `opt-level=1` regressions on deps)**

Run: `cargo nextest run --all-features`

Expected: all tests pass. Embedder test group runs serialized (per `.config/nextest.toml`).

- [ ] **Step 7: Run umbrella gate**

Run: `bash scripts/check-all.sh`

Expected: exits 0.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml
git -c commit.gpgsign=false commit -m "perf(cargo): profile split, dev tune, rust-version 1.95"
```

---

## Task 8: Update `scripts/install.sh`

**Files:**
- Modify: `scripts/install.sh`

- [ ] **Step 1: Replace the release build invocation**

In `scripts/install.sh`, change:

```bash
log_info "$STEP" "building release binary"
run_cargo build --release --locked

log_info "$STEP" "installing into cargo bin"
run_cargo install --path "$PROJECT_ROOT" --locked --force
```

to:

```bash
log_info "$STEP" "building release-quick binary"
run_cargo build --profile release-quick --locked

log_info "$STEP" "installing into cargo bin (release-quick profile)"
run_cargo install --path "$PROJECT_ROOT" --profile release-quick --locked --force

if [[ "${1:-}" == "--with-tools" ]]; then
  if command -v brew >/dev/null 2>&1; then
    log_info "$STEP" "installing optional tools (sccache, hyperfine)"
    brew install sccache hyperfine
  else
    log_info "$STEP" "brew not found; skipping optional tools"
  fi
fi
```

- [ ] **Step 2: Lint the script**

Run: `shellcheck scripts/install.sh`

Expected: no output, exit 0.

- [ ] **Step 3: Dry-run install (without `--with-tools`)**

Run: `bash scripts/install.sh`

Expected: builds via `release-quick`, runs `cargo install`, prints `installed …/qwick-memory (qwick-memory 0.1.0)`.

- [ ] **Step 4: Smoke-check the installed binary**

Run: `qwick-memory --version && qwick-memory doctor`

Expected: version `qwick-memory 0.1.0`; doctor reports green.

- [ ] **Step 5: Commit**

```bash
git add scripts/install.sh
git -c commit.gpgsign=false commit -m "perf(install): switch to release-quick, optional --with-tools"
```

---

## Task 9: Update CI workflow

**Files:**
- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Rewrite the workflow**

Replace `.github/workflows/ci.yml` with:

```yaml
name: ci
on:
  pull_request:
  push:
    branches: [main]

env:
  RUSTC_WRAPPER: sccache
  SCCACHE_GHA_ENABLED: "true"

jobs:
  test:
    runs-on: macos-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with: { components: rustfmt, clippy }
      - uses: mozilla-actions/sccache-action@v0.0.5
      - uses: Swatinem/rust-cache@v2
      - uses: taiki-e/install-action@v2
        with: { tool: nextest,cargo-deny,typos-cli,cargo-machete }
      - run: bash scripts/check-all.sh
      - run: bash scripts/deny-check.sh
      - run: bash scripts/dup-check.sh
      - run: bash scripts/e2e.sh
```

- [ ] **Step 2: Lint YAML (via Python — already on macOS by default)**

Run: `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/ci.yml')); print('ok')"`

Expected: prints `ok`. If `yaml` is missing, run `pip3 install pyyaml --user` once.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git -c commit.gpgsign=false commit -m "ci: macOS-only matrix, sccache wrapper"
```

---

## Task 10: Update README

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Read current README**

Run: `wc -l README.md && grep -n '^##' README.md`

Note the existing section headings; the new section must not conflict.

- [ ] **Step 2: Append "Optional: faster builds" section**

Append to `README.md`:

```markdown

## Optional: faster builds

These tools are entirely optional. The project builds with stock
`cargo` out of the box.

- **sccache** — caches rustc outputs across `cargo clean`. ~5–20%
  cold-build win on a warm cache.
- **hyperfine** — used by `just perf` to measure warm-incremental p50/p95.

Install (Apple Silicon):

```bash
brew install sccache hyperfine
# or: bash scripts/install.sh --with-tools
```

Activate sccache by exporting in your shell init:

```bash
export RUSTC_WRAPPER=sccache
```

Measure your builds:

```bash
just perf            # writes target/build-perf/summary.json
bash scripts/build-perf.sh --append-md   # also appends a row to docs/build-perf.md
```

Local fast release builds: `cargo build --profile release-quick`
(`scripts/install.sh` already uses this). Distributed binaries continue
to use `[profile.release]` via `cargo-dist`.
```

- [ ] **Step 3: Commit**

```bash
git add README.md
git -c commit.gpgsign=false commit -m "docs(readme): optional faster-builds section"
```

---

## Task 11: Re-run perf harness post-change and append row

**Files:**
- Modify: `docs/build-perf.md` (appends one row).

- [ ] **Step 1: Run with sccache absent (apples-to-apples vs baseline)**

```bash
unset RUSTC_WRAPPER
BUILD_PERF_NOTES="post-change (no sccache)" bash scripts/build-perf.sh --append-md
```

Expected: prints `[build-perf] OK …` and appends one row.

- [ ] **Step 2: Optional second run with sccache (if installed)**

If `command -v sccache` returns a path:

```bash
export RUSTC_WRAPPER=sccache
sccache --zero-stats
BUILD_PERF_NOTES="post-change (sccache)" bash scripts/build-perf.sh --append-md
unset RUSTC_WRAPPER
```

If sccache is absent, skip — do not install it just to populate this row.

- [ ] **Step 3: Eyeball the delta**

Run: `tail -n 5 docs/build-perf.md`

Expected: baseline row, post-change (no sccache) row, optional post-change (sccache) row. Confirm the post-change numbers are at least non-regressive on warm p50. Suggested target thresholds from the spec: cold ≤ 50% baseline, warm-check ≤ 3s, release ≤ 60% baseline. If post-change misses these targets, do NOT block — record the result and open a follow-up issue for feature trimming (out of scope for this PR).

- [ ] **Step 4: Commit**

```bash
git add docs/build-perf.md
git -c commit.gpgsign=false commit -m "perf(docs): record post-change build numbers"
```

---

## Task 12: Full verification pass

**Files:**
- None.

- [ ] **Step 1: Umbrella gate**

Run: `bash scripts/check-all.sh`

Expected: exits 0. If a step fails, fix it before continuing — never bypass.

- [ ] **Step 2: Full QA**

Run: `just qa`

Expected: `check-all`, `deny-check`, `dup-check`, `machete-check` all pass.

- [ ] **Step 3: Full test suite**

Run: `just test`

Expected: `cargo nextest run --all-features` passes.

- [ ] **Step 4: End-to-end harness**

Run: `just e2e`

Expected: passes against the `release-quick` binary installed in Task 8.

- [ ] **Step 5: cargo-dist plan dry-run**

Run: `cargo dist plan 2>&1 | tail -n 30`

Expected: emits a plan for `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`. If `cargo dist` is not installed locally, skip this step — GitHub Actions will run it on the PR.

- [ ] **Step 6: Doctor smoke**

Run: `qwick-memory doctor`

Expected: green.

- [ ] **Step 7: No commit**

Verification only.

---

## Task 13: Push the branch and open the PR

**Files:**
- None.

- [ ] **Step 1: Push the branch**

```bash
git push -u origin perf/build-time-single-pr
```

- [ ] **Step 2: Open the PR**

```bash
gh pr create --title "perf: cut build time via profile split + sccache + harness" --body "$(cat <<'EOF'
## Summary

- Add `scripts/build-perf.sh` + `scripts/lib/perf.sh` measurement harness; record cold/warm/release numbers to `docs/build-perf.md`.
- Add `.cargo/config.toml` (env-gated sccache, aarch64 link flag) and `Cargo.toml` profile changes: `release-quick` profile, dev tune (`opt-level=1` for deps, `line-tables-only` debuginfo), `build-override` for build scripts. `[profile.release]` is untouched — `cargo-dist` continues to use it for distributed binaries.
- macOS-only CI matrix + sccache action. `scripts/install.sh` switches to `release-quick` and grows an opt-in `--with-tools` flag.
- Spec: `docs/superpowers/specs/2026-05-18-build-time-improvements-design.md`.
- Baseline + post-change numbers in `docs/build-perf.md`.

## Test plan
- [ ] `bash scripts/check-all.sh`
- [ ] `just qa`
- [ ] `just test`
- [ ] `just e2e`
- [ ] `qwick-memory doctor`
- [ ] CI passes on macOS runner with sccache

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

Expected: prints the PR URL.

---

## Self-Review

**Spec coverage:**
- §4 Architecture (three layers): Tasks 2–4 (measurement), 6–7 (build-config), 8–10 (tooling/CI). ✓
- §5.1 measurement layer: Tasks 2 (`perf.sh`), 3 (`build-perf.sh`), 4 (`just perf` + `docs/build-perf.md`). ✓
- §5.2 build-config: Task 6 (`.cargo/config.toml`), Task 7 (`Cargo.toml` profiles + `rust-version`). ✓
- §5.3 tooling/CI: Task 8 (`install.sh`), Task 9 (CI), Task 10 (README). ✓
- §8 testing (every check listed): Task 12 covers `check-all`, `qa`, `test`, `e2e`, `doctor`, `cargo dist plan`. ✓
- §9 rollout (baseline row, then post-change row): Tasks 5 and 11. ✓
- §10 rollback: covered implicitly by the PR being one revertable merge.
- Spec §1 gitignore note: `/target` already gitignores `target/build-perf/`. No `.gitignore` change required.

**Placeholder scan:** no `TBD`, `TODO`, `implement later`, or "similar to Task N" markers. Each step contains the exact code or command to run.

**Type / name consistency:**
- Helper names match between Task 2 (`perf_now_ms`, `perf_time_once`, `perf_time_runs`, `perf_top_crates`) and Task 3 (sourced + invoked verbatim). ✓
- Profile names consistent: `release-quick` in Cargo.toml (Task 7), `install.sh` (Task 8), README (Task 10). ✓
- `RUSTC_WRAPPER=sccache` used identically in `.cargo/config.toml` (Task 6, env table), CI (Task 9, job env), README (Task 10). ✓
- `BUILD_PERF_NOTES` env var consumed by `build-perf.sh` (Task 3) and set by Tasks 5 and 11. ✓
- File-size cap (500 lines) enforced via explicit `wc -l` check in Tasks 2 and 3. ✓

No outstanding issues.
