# Build Time Improvements — Design Spec

- **Date:** 2026-05-18
- **Status:** Approved (single PR)
- **Author:** falconiere (`hello@falconiere.io`)
- **Audience:** qwick-memory contributors

## 1. Goal

Reduce local build wall-clock for cold, warm-incremental, and release
builds on macOS Apple Silicon. Ship measurement infrastructure in the
same PR so the impact is provable and future regressions are catchable.

## 2. Non-Goals

- Feature trimming of heavy crates (`lancedb`, `fastembed`). Deferred to
  follow-up issues; needs per-feature targeted testing.
- Workspace split (carving `qwick-memory` into sub-crates). Deferred to
  a separate architectural spec.
- Nightly-only tooling (`cranelift` codegen backend, parallel frontend).
  Violates the zero-setup contributor constraint.
- Linux / x86_64 tuning. macOS Apple Silicon is the only supported dev
  target.

## 3. Constraints

1. **Zero-setup for contributors.** Cloning the repo and running
   `cargo build` works without installing `sccache`, `hyperfine`, or any
   other tool. Optional tooling is opt-in via `scripts/install.sh`.
2. **Release profile preserved.** `[profile.release]` keeps `lto = "fat"`,
   `codegen-units = 1`, `strip = "symbols"`. `cargo-dist` depends on
   these for distributed binaries.
3. **macOS Apple Silicon only.** Optimizations target
   `aarch64-apple-darwin`; CI matrix loses `ubuntu-latest`.
4. **All five binding rules + all gates remain green.** No `src/` files
   touched, no `#[allow(...)]` overrides, no script bypass.
5. **Single PR.** All changes ship together. Each change is additive or
   opt-in so revert is one `git revert`.

## 4. Architecture

Three logical layers, all delivered in one PR:

```
┌─────────────────────────────────────────────────┐
│ Measurement layer                               │
│ - scripts/build-perf.sh, scripts/lib/perf.sh    │
│ - docs/build-perf.md (committed history)        │
└──────────────────┬──────────────────────────────┘
                   │ captures before/after numbers
┌──────────────────▼──────────────────────────────┐
│ Build-config layer                              │
│ - .cargo/config.toml                            │
│ - Cargo.toml profile + rust-version updates     │
└──────────────────┬──────────────────────────────┘
                   │ applied by cargo automatically
┌──────────────────▼──────────────────────────────┐
│ Tooling / CI layer                              │
│ - scripts/install.sh (optional sccache install) │
│ - .github/workflows/ci.yml (macOS-only + sccache)│
└─────────────────────────────────────────────────┘
```

## 5. Components

### 5.1 Measurement layer

#### `scripts/build-perf.sh` (new)

Driver. Runs three scenarios; writes machine-readable and human-readable
output to `target/build-perf/` (gitignored).

| Scenario | Command | Tool |
|---|---|---|
| Cold | `cargo clean && cargo build` | `cargo build --timings=html,json` + wall-clock |
| Warm | touch `src/lib.rs`, `cargo build` (×5) | `hyperfine` if present, else `time` |
| Release | `cargo clean && cargo build --release` | `cargo build --timings=html,json` + wall-clock |

Outputs:
- `target/build-perf/cargo-timings-cold.html`
- `target/build-perf/cargo-timings-release.html`
- `target/build-perf/summary.json` —
  `{ commit, date, rustc, cargo, sccache: bool, cold_s, warm_p50_s,
     warm_p95_s, release_s, top10_crates: [{name, version, duration_s}] }`

Parses `cargo-timings.json` for top-10 unit durations via `jq`. No custom
parser.

#### `scripts/lib/perf.sh` (new)

Shared helpers: `time_ms`, `run_hyperfine_or_time`, `write_summary_json`,
`format_md_row`. Sourced by `build-perf.sh`.

#### `docs/build-perf.md` (new)

Markdown history. One row per measured run.

| Date | Commit | Cold | Warm p50 | Warm p95 | Release | sccache | Notes |
|---|---|---|---|---|---|---|---|

PR commit message references the baseline row and the post-change row.

#### Integration

- `justfile` gets a `perf:` target → `bash scripts/build-perf.sh`.
- Not added to `scripts/check-all.sh` (too slow).
- `.gitignore` adds `target/build-perf/`.

#### Optional tooling

`hyperfine` install is best-effort. If absent, `scripts/build-perf.sh`
falls back to `time` and writes a `warm_p50_s` only (no p95). Documented
in script header.

### 5.2 Build-config layer

#### `.cargo/config.toml` (new)

```toml
# Respect RUSTC_WRAPPER from environment.
# Contributors without sccache: zero impact.
# Contributors with sccache: export RUSTC_WRAPPER=sccache in their shell.

[target.aarch64-apple-darwin]
rustflags = ["-C", "link-arg=-Wl,-dead_strip"]

[env]
SCCACHE_CACHE_SIZE = { value = "20G", force = false }
```

Rationale:
- No hard `rustc-wrapper = "sccache"` line — that would fail builds on
  machines without sccache, violating zero-setup.
- `-Wl,-dead_strip` reclaims size on the Apple linker. Cheap, no
  correctness impact.
- `SCCACHE_CACHE_SIZE` only takes effect when sccache is invoked.

#### `Cargo.toml` profile changes

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

# [profile.release] UNTOUCHED — cargo-dist depends on lto=fat, etc.

[profile.release-quick]
inherits = "release"
lto = "thin"
codegen-units = 16
strip = "none"
```

Rationale:
- `opt-level = 1` for deps → tests and dev builds run dramatically
  faster on hot loops (fastembed, lancedb) while crate-local code stays
  `opt-level = 0` for snappy rebuilds.
- `line-tables-only` cuts debuginfo size and link time without losing
  backtraces.
- `split-debuginfo = "unpacked"` is macOS default but pinned explicit so
  it can’t silently change.
- `build-override` profiles speed up build scripts (notably kuzu’s C++
  bindgen).
- `release-quick` is a fast local profile. `scripts/install.sh` uses it.
  `cargo-dist` continues to use `release`.

#### `rust-version` bump

`rust-version = "1.78"` → `"1.95"` (matches installed `rustc 1.95.0`).
Reflected in README. Verified via `rustc --version` before commit.

### 5.3 Tooling / CI layer

#### `scripts/install.sh` updates

- Detect `brew`. If present and `sccache` / `hyperfine` missing, print a
  one-line note. Install only when invoked with `--with-tools` flag (no
  silent install).
- Switch `cargo build --release --locked` → `cargo build --profile
  release-quick --locked`.
- Switch `cargo install --path . --locked --force` →
  `cargo install --path . --profile release-quick --locked --force`.
  `release-quick` is never the default profile; the flag is always
  explicit.

#### `.github/workflows/ci.yml` updates

- `matrix.os`: drop `ubuntu-latest`, keep only `macos-latest`.
- Add `mozilla-actions/sccache-action@v0.0.5` before `Swatinem/rust-cache@v2`.
- Set job env: `RUSTC_WRAPPER: sccache`, `SCCACHE_GHA_ENABLED: "true"`.
- Keep `Swatinem/rust-cache@v2` — caches `target/` independently of
  sccache. The two layers compose.

#### `README.md` updates

New section: **Optional: faster builds**.

- Install: `brew install sccache hyperfine` (or `scripts/install.sh
  --with-tools`).
- Export in shell: `export RUSTC_WRAPPER=sccache`.
- Run `just perf` to measure.

## 6. Data Flow

1. Contributor runs `cargo build` →
   `.cargo/config.toml` honored (`RUSTC_WRAPPER` if exported, link flag
   applied). Cargo uses `[profile.dev]` settings. Build scripts use
   `build-override`.
2. Contributor runs `just perf` → `scripts/build-perf.sh` runs three
   scenarios, writes `target/build-perf/summary.json` + appends row to
   `docs/build-perf.md`.
3. CI: GH Actions checkout → `sccache-action` → `rust-cache` →
   `check-all.sh`. Wrapper is set in job env, so all `cargo` invocations
   route through sccache.
4. `scripts/install.sh` → builds `release-quick`, installs via
   `cargo install`.

## 7. Error Handling

- `scripts/build-perf.sh` is bash with `set -euo pipefail`. Missing
  `hyperfine` → fall back to `time`. Missing `jq` → script exits with a
  clear message (jq is a hard dep of the harness; install via brew).
- `.cargo/config.toml` syntax errors → cargo emits its own error. No
  custom handling.
- CI sccache failure → action continues without caching; build still
  succeeds.

## 8. Testing

| Check | Command | Layer |
|---|---|---|
| Format | `scripts/fmt-check.sh` | `check-all.sh` |
| Type | `scripts/type-check.sh` | `check-all.sh` |
| Lint | `scripts/lint-check.sh` | `check-all.sh` |
| Test placement | `scripts/test-placement-check.sh` | `check-all.sh` |
| Bypass scan | `scripts/no-bypass-check.sh` | `check-all.sh` |
| Module size | `scripts/module-size-check.sh` | `check-all.sh` (covers new scripts ≤500 lines) |
| Mirror | `scripts/tests-mirror-check.sh` | `check-all.sh` |
| Typos | `scripts/typos-check.sh` | `check-all.sh` |
| Cargo deny | `scripts/deny-check.sh` | `just qa` |
| Dup check | `scripts/dup-check.sh` | `just qa` |
| Cargo machete | `scripts/machete-check.sh` | `just qa` |
| Full suite | `cargo nextest run --all-features` | `just test` |
| E2E | `scripts/e2e.sh` | `just e2e` |
| Doctor | `qwick-memory doctor` | smoke test |
| Shell lint | `shellcheck scripts/build-perf.sh scripts/lib/perf.sh` | manual / pre-merge |
| Bench | `just perf` | manual; output committed to `docs/build-perf.md` |

Additional bespoke checks:
- `cargo build` works **with** `RUSTC_WRAPPER=sccache` and **without**.
- `cargo build --release` produces a working binary; `qwick-memory
  --version` matches `Cargo.toml`.
- `cargo build --profile release-quick` produces a working binary;
  `scripts/e2e.sh` passes against it.
- `cargo-dist plan` dry-run still emits the four expected targets.
- CI passes on macOS-only matrix.

## 9. Rollout

Single PR titled `perf: cut build time via profile split + sccache + harness`.

Sequence within the PR:
1. Add `scripts/build-perf.sh`, `scripts/lib/perf.sh`, run once on
   current `main`, commit `docs/build-perf.md` with the **baseline row**.
2. Add `.cargo/config.toml`, `Cargo.toml` profile changes,
   `rust-version` bump.
3. Update `scripts/install.sh` and `.github/workflows/ci.yml`.
4. Update `README.md`.
5. Run `just perf` again. Append **post-change row** to
   `docs/build-perf.md`. Commit.
6. Open PR; cargo-dist `plan` runs as part of CI.

## 10. Rollback

`git revert <merge-sha>`. All changes are additive or opt-in, so revert
restores prior behavior cleanly. No data migration, no state.

## 11. Open Questions

- Exact target for "good enough": baseline numbers from §9 step 1 will
  define stop criteria. Suggested thresholds (refine after baseline):
  cold ≤ 50% of baseline, warm-check ≤ 3s on `src/lib.rs` touch,
  release ≤ 60% of baseline.
- If post-change numbers miss the targets, a follow-up issue opens for
  feature trimming (§2) — not part of this PR.

## 12. References

- Project rules: `/Users/falconiere/Projects/qwick-memory/CLAUDE.md`
- Gate scripts: `scripts/check-all.sh`
- Cargo profiles:
  https://doc.rust-lang.org/cargo/reference/profiles.html
- cargo build timings:
  https://doc.rust-lang.org/cargo/reference/timings.html
- sccache: https://github.com/mozilla/sccache
- hyperfine: https://github.com/sharkdp/hyperfine
- Apple ld: `ld(1)` man page, Xcode 15+ linker notes
