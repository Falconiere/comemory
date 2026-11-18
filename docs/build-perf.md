# Build Performance

## Quality gates

`just check` runs cheap repository checks first, then Clippy over all targets
and features (which also type-checks), then the CLI and HTTP documentation
checks. It stops at the first failed gate and reports elapsed seconds per gate
and for the complete run. `scripts/type-check.sh` remains available for a
standalone type-check; the umbrella does not repeat it before Clippy.

Documentation regeneration and retrieval evaluation build only the dev
`comemory` binary. Neither needs optimization or LTO; release and installation
profiles keep their existing settings. Cargo supplies the executable path,
including when `CARGO_TARGET_DIR` changes the artifact directory.

Migration immutability records each SQL file's first tag/path during one scan
of release history, rather than searching the tags again for each file. A local
run over 94 tags and 35 migrations fell from 84 seconds to 3.8 seconds (individual
runs under concurrent build load). The first-release anchor and the union of
shipped files are preserved across the legacy directory move. Run
`bash scripts/test-migration-check.sh` to exercise edits, moves, conflicting
legacy/current paths, unreleased files, and deletion from later releases using
real SQL files and disposable Git history; CI runs this regression too.

The binary target has no unit tests or benchmarks, so its empty test/bench
harnesses are disabled. CLI integration tests still build and exercise the
real binary. Keep the behavior tests; do not add tests that only assert script
spelling, gate order, or the shape of a configuration file. Verify gate changes
with the actual checks and a disposable failing input.

Use focused tests during development, for example:

```bash
cargo nextest run --test cli__save --all-features
```

Run `just check` before pushing; `just test` runs the complete behavior suite.
The broader `just qa` and CI coverage/security gates remain separate. Vendored
`scripts/guardrails/` is unchanged.

## Build history

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
| v0.7    | ~10.5 MB       | added the `serve` web SPA, embedded + gzip-compressed (removed again in v0.18 — `serve` is API-only) |
| v0.8    | ~10.7 MB       | edition 2024 + fat-LTO release profile; no new runtime code (measured 10.74 MB, `aarch64-apple-darwin` v0.8.2) |

The v0.2 rewrite dropped the in-process embedder, vector DB, and graph DB. The
web viewer that briefly followed (v0.7–v0.17) was the only meaningful weight
back; `serve` is API-only again, so the embedded SPA is gone.
