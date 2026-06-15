# Benchmarks

`scripts/bench.sh` (`just bench`) runs `cargo bench --all-features` and writes
the criterion output to `docs/bench/latest.md`, plus the HTML report under
`target/criterion/`. Both are regenerated on every run and are git-ignored
(numbers are hardware-variable, per the Reproducibility note below); only this
`README.md` is tracked.

Heavy fixtures (the seeded SQLite corpus, the on-disk git repo) are built
**once** before the criterion timer starts, so headline numbers measure the hot
path, not setup.

## What we track

Four `[[bench]]` targets (`harness = false`), each driving the real public
`comemory` hot paths over a synthesized corpus seeded through the store API
(`memory_row::insert` + `vector::insert_memory`, `code_row::insert` +
`fts::index_code` + `vector::insert_code`) — deterministic BYO vectors
(SHA-256-derived, memory dim 1024 / code dim 768), no embedder, no network.

- **`retrieval`** — the end-to-end memory pipeline (`pipeline::search`: lexical
  ladder + `sqlite-vec` ANN + RRF fuse + rerank + diversify) and the code route
  (`route_code` → `rerank_code`), swept over corpus sizes 100 / 1 000 / 10 000.
- **`indexing`** — `ast::extract` (tree-sitter extract + cAST chunk) over a real
  comemory source file, then the per-symbol write fan-out (`code_row::insert` +
  `fts::index_code` + `vector::insert_code`) into a fresh migrated DB — the same
  three public calls the `index-code` path makes.
- **`store`** — the SQLite + `sqlite-vec` primitives in isolation over a
  ≥2 000-row corpus with vectors seeded: vec0 KNN (`knn_memory` / `knn_code`),
  FTS5 BM25 (`search_memory` / `search_code`), and the recursive-CTE supersedes
  walk (`supersedes_chain`). KNN is asserted non-empty before timing.
- **`graph`** — the graph kernels: deterministic `pagerank` over a synthetic
  edge list (node sweep), `extract_imports` + `PathIndex::resolve` over real
  source, and the git-backed miners (`mine_cochange`, `materialize`) over a real
  on-disk git repo built at setup (≥50 commits, ≥2 files each, so co-change
  yields non-zero pairs).

## Reproducibility

Numbers vary across hardware and toolchain version. Re-run the bench on the
same host before comparing.
