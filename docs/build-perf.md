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
| 2026-05-18T16:48:28Z | 51ae8d9 | 702 | 4.12 | 4.12 | n/a | absent | baseline (pre-change, partial; release deferred to post-change run) |
| 2026-05-18T17:29:33Z | 214fa44 | 420.640 | 2.856 | 2.856 | 1228.009 | absent | post-change (no sccache) |

## v0.2 refactor — size measurements

| Stage | Profile | Size (MB) | Date |
|-------|---------|-----------|------|
| Baseline (v0.1)          | release | 116.8 | 2026-06-08 |
| Task 1 (`panic="abort"`) | release | 97.1 | 2026-06-08 |
| Task 18 (deps deleted)   | release | 8.2 | 2026-06-08 |
| Task 20 (final, unstripped) | release | 8.2 | 2026-06-08 |
| Task 20 (final, stripped)   | release | 8.2 | 2026-06-08 |

## Binary size over time

| Version | Release binary | Notes |
|---------|---------------:|-------|
| v0.1    | ~117 MB        | bundled fastembed + lancedb + kuzu |
| v0.2    | ~8 MB          | one SQLite file, BYO vectors, trimmed tree-sitter set |
| v0.7    | ~10.5 MB       | adds the `serve` web SPA, embedded + gzip-compressed |
| v0.8    | ~10.7 MB       | edition 2024 + fat-LTO release profile; no new runtime code (measured 10.74 MB, `aarch64-apple-darwin` v0.8.2) |

The v0.2 rewrite dropped the in-process embedder, vector DB, and graph DB. The
web viewer added since is the only meaningful weight back, and it's
gzip-compressed in the binary.
