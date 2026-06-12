# Changelog

## 0.4.0 — 2026-06-11 (M3 code graph + code-aware retrieval)

The code layer becomes a graph. A v6 migration adds code-graph edges,
PageRank/chunk columns, and a `code_feedback` table; the database
auto-migrates v5 → v6 on first open and markdown files are untouched.

### Added
- **`comemory search-code`** — ranked code search (weighted BM25 + a
  thresholded ANN leg fused with RRF, chunk→parent coalesce) reranked
  by four graph priors (PageRank, recency, working-set affinity,
  feedback), with per-query logging and a `query_id` for feedback.
- **Code-graph edges**: `co_changed` (mined from git history with a
  sliding window, mega-commit guard, and resumable cursor) and
  `imports` (conservative per-language import resolution for rust /
  typescript / javascript / python / go).
- **Deterministic weighted PageRank** over the code graph, materialized
  onto `code_symbols.rank_score` by `comemory index-code`.
- **cAST chunking**: oversized symbols split into child rows at AST
  boundaries so large definitions stay retrievable.
- `comemory feedback` accepts code targets and records per-query
  provenance for code results.
- `comemory context` ranks referenced code symbols by graph priors.
- `comemory ast` / the extractor now capture `pub` / `export`-modified
  definitions.
- Config: code BM25 weights and `COMEMORY_RETRIEVAL_CODE_THRESHOLD`
  (re-consumed for the `search-code` ANN leg), plus configurable
  rank / prune / tune constants and matching `COMEMORY_*` env vars.

### Changed
- `comemory eval` replays each golden query's originating repo / kind
  filters so measurement matches production retrieval.
- The learning loop logs search filters and source; mining ignores
  code searches so reformulation expansions stay memory-scoped.
- `comemory index-code` mines co-change + imports and materializes
  PageRank as part of the indexing pass.

### Fixed
- Retrieval skips working-set discovery when the context query returns
  no hits.
- Feedback resolves chunk ids to their parent symbol identity.
- PageRank edge load is ordered by logical graph keys for determinism.

## 0.3.0 — 2026-06-11 (Rank-blend retrieval + learning loop)

Two milestones in one release: M1 (rank-blend core, PR #4) and M2
(learning loop, PR #5). The database auto-migrates v2/v3/v4 → v5 on
first open; markdown files are untouched.

### Breaking
- `comemory feedback` requires the `q-<yyyymmdd>-<8hex>` query id
  printed by `comemory search` / `comemory context`; free-form ids are
  rejected.
- `score_parts.rrf` in `--json` output is now max-normalized relevance
  in `[0, 1]` (pool max → 1.0), no longer the raw fused score. The
  product invariant `final_score == rrf × activation × feedback ×
  quality × supersede` still holds.
- `comemory prune` reports by default; pass `--apply` to soft-delete.
- The unused `search_stats` table is dropped and the unconsumed
  `COMEMORY_RETRIEVAL_CODE_THRESHOLD` knob is removed.
- `comemory gc` now loads the layered config and errors on invalid
  values, like every other subcommand.

### Added
- **Learning loop**: every search logs to `retrieval_log` and emits a
  `query_id`; `comemory feedback <query_id>` records per-query
  provenance in `feedback_events`.
- `comemory eval` — recall@k / MRR against golden pairs harvested from
  feedback and/or a `--golden` YAML file (runs with tracking off, so
  measurement never pollutes its own signal).
- `comemory mine` — mines failed→reworded query pairs into
  `query_expansions`; the lexical ladder gains a learned-expansion
  tier (support ≥ 2, ≤ 2 expansions per term), surfaced via the new
  `tier` field.
- `comemory tune` — deterministic 81-point grid search over rrf_k,
  decay, MMR lambda, and BM25 weights; `--apply` rewrites config.toml
  atomically only on strict improvement (requires ≥ 10 golden pairs).
- `comemory search --kind` filters results to one memory kind.
- `comemory save --supersedes` records supersession; superseded
  memories rank with a 0.2 penalty and prune respects a 7-day grace.
- Save-time near-duplicate warning (64-bit SimHash, Hamming ≤ 8).
- Config: `[retrieval] bm25_weights` (body, tags), `[rank]` decay /
  prior_clamp / mmr_lambda, `[prune]` learning_retention_days, plus
  matching `COMEMORY_*` env vars.

### Changed
- Custom FTS5 `identifier` tokenizer: camelCase / snake_case / digit
  splitting with colocated whole tokens and diacritic folding —
  `VecDimMismatch` and "dim mismatch" reach each other.
- Retrieval is two-stage: weighted-BM25/ANN candidates (tiered
  relaxation ladder) → deterministic rerank (ACT-R activation,
  Beta-smoothed feedback, quality, supersede priors on a normalized
  relevance scale) → SimHash near-dup collapse + MMR diversity.
- `comemory rebuild` preserves learning state (feedback counters,
  events, query log, mined expansions) alongside the code index.
- `comemory gc` evicts learning telemetry older than 90 days
  (configurable); counters and expansions never expire.

## 0.2.0-rc.1 — 2026-06-09 (Pre-release dry-run)

Pre-release exercising the cargo-dist release pipeline before the
final 0.2.0 cut. No source changes vs. 0.2.0. Pre-release tag does
not update the Homebrew tap.

## 0.2.0 — 2026-06-09 (Lightweight refactor)

### Breaking
- Dropped `comemory serve` (axum web UI).
- Dropped the in-process embedder. Embedding is now the caller's
  responsibility; pass vectors via `--vector` or `--vector-stdin`.
- `~/.comemory/lancedb/` and `~/.comemory/kuzu/` directories are
  ignored. Run `comemory rebuild` to populate `~/.comemory/comemory.db`
  from `memories/*.md`.
- `--lang` on `comemory ast` now accepts only `rust`, `typescript`,
  `javascript`, `python`, `go`.

### Added
- `comemory ingest-code` reads pre-embedded JSONL into `code_symbols`
  and `code_vec`.
- `comemory rebuild` drops and reconstructs `comemory.db` from
  markdown.
- `scripts/comemory-embed.sh` — sample Ollama wrapper for the BYO
  contract.

### Changed
- Single `~/.comemory/comemory.db` SQLite file backs all storage
  (memories, FTS5, sqlite-vec, edges, stats).
- Release binary size: 117 MB → ~8 MB (after dropping the in-process
  embedder/lancedb/kuzu and trimming `ast-grep-language` to the
  rust/typescript/javascript/python/go tree-sitter parsers).
