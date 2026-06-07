# Benchmarks

Run `just bench` to regenerate `docs/bench/latest.md`. The harness is criterion
0.5; results include mean, median, and p99 latency.

## What we track

Both benches construct their heavy setup (fixture, embedder, LanceDB
connection, FTS handle, query embedding) **once** before the criterion
timer starts so the headline numbers measure actual work, not init.

- `save_end_to_end` — `comemory save` cost: markdown write + nomic
  embedding + `MemoryIndex::upsert` + FTS5 insert. Each iteration reuses
  the same embedder + LanceDB + FTS handle but **varies the body per
  iteration** via an atomic counter, so the resulting `memory_id` is
  unique each time. The measurement therefore reflects the cold-insert
  path (new markdown file, `merge_insert` against an id never seen
  before, fresh FTS row) rather than the steady-state overwrite. Watch
  this when changing the embed or upsert path.
- `search_vector_only` — dense-only baseline via
  `retrieval::fuse::search_memory_fused_with_fts(idx, None, ...)`. Passing
  `None` for the FTS handle short-circuits the BM25 path so the
  measurement reflects pure dense retrieval through the unified entry
  point (there is no separate `hybrid::search_memory` anymore).
- `search_fused_rrf_cold_fts` — `retrieval::fuse::search_memory_fused`
  exactly as `comemory search` invokes it: every iteration re-opens the
  FTS5 SQLite file. The delta vs `search_vector_only` is the production
  cost of fusion + the FTS5 round-trip + the per-call SQLite connection
  open.
- `search_fused_rrf_warm_fts` —
  `retrieval::fuse::search_memory_fused_with_fts` with a pre-opened
  `Fts` handle reused across iterations. The delta vs
  `search_fused_rrf_cold_fts` isolates the SQLite open cost so callers
  evaluating "should I cache the FTS handle?" have a number to point at.

## Reproducibility

Numbers vary across hardware, ONNX runtime version, and cold-vs-warm fastembed
model cache. Re-run the bench on the same host before comparing.
