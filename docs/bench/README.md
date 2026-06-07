# Benchmarks

Run `just bench` to regenerate `docs/bench/latest.md`. The harness is criterion
0.5; results include mean, median, and p99 latency.

## What we track

- `save_end_to_end` — `comemory save` cost: markdown write + nomic embedding +
  `MemoryIndex::upsert` + FTS5 insert. Watch this when changing the embed or
  upsert path.
- `search_vector_only` — `retrieval::hybrid::search_memory` baseline.
- `search_fused_rrf` — `retrieval::fuse::search_memory_fused` (dense ⊕ BM25
  with Reciprocal Rank Fusion). The delta vs `search_vector_only` is the
  latency cost of fusion + the FTS5 round-trip.

## Reproducibility

Numbers vary across hardware, ONNX runtime version, and cold-vs-warm fastembed
model cache. Re-run the bench on the same host before comparing.
