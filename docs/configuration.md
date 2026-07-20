# Configuration

comemory's settings are layered: built-in defaults → an optional `config.toml`
→ environment variables (the last wins). The CLI also takes the global
`--data-dir` and `--json` flags (see [CLI reference](cli-reference.md)).

## Environment variables

| Variable | Purpose | Default |
|----------|---------|---------|
| `COMEMORY_DATA_DIR` | Root data directory (`memories/` + `comemory.db`). | `~/.comemory` |
| `COMEMORY_INDEXING_AUTO_REINDEX` | `lazy` \| `hook` \| `off` — automatic code-index refresh. See [Keep the code index fresh](guides/auto-reindex.md). | `lazy` |
| `COMEMORY_RETRIEVAL_TOP_K` | Results returned by the hybrid router (also the default page size for `search` / `search-code` / `context`). | `12` |
| `COMEMORY_RETRIEVAL_MAX_PAGE_WINDOW` | Maximum depth pagination can reach into the ranked list; `has_more` is forced false at this ceiling. Validated `> 0`. | `200` |
| `COMEMORY_RETRIEVAL_MEMORY_THRESHOLD` | Minimum cosine similarity for the memory table. | `0.55` |
| `COMEMORY_RETRIEVAL_CODE_THRESHOLD` | Minimum cosine similarity for the code table (ANN leg of `search-code`, range `[0.0, 1.0]`). | `0.50` |
| `COMEMORY_RETRIEVAL_RRF_K` | RRF fusion constant for hybrid scoring. | `60.0` |
| `COMEMORY_RETRIEVAL_BM25_WEIGHTS` | `"body,tags"` BM25 column weights for `memory_fts` (both finite ≥ 0, at least one > 0). | `1.0,3.0` |
| `COMEMORY_RETRIEVAL_CODE_BM25_WEIGHTS` | `"symbol,snippet,path_tokens"` BM25 column weights for `code_fts` (all finite ≥ 0, at least one > 0). | `2.0,1.0,1.5` |
| `COMEMORY_LEARNING_RETENTION_DAYS` | `comemory gc` retention window (days) for raw `retrieval_log` + `feedback_events` rows; aggregated `feedback` counters and mined `query_expansions` never expire. | `90` |
| `COMEMORY_TUNE_MIN_GOLDEN` | Test hook lowering `comemory tune` / `comemory bandit` minimum-golden-pairs floor; not a tuning knob. | `10` |
| `COMEMORY_REINFORCE_SEARCH_EDIT_DAYS` | Lookback (days) for search→edit auto-reinforcement: a memory that appeared on a recent `search`/`context` page earns `auto_search_edit` provenance when a referenced file is touched. Must be `≥ 1`. | `7` |
| `COMEMORY_GIT_AUTO_SYNC` | `true`/`1` to enable best-effort git commit + push after a save. | `false` |
| `COMEMORY_EMBED_HINT` | Free-form identifier of the embedder you used (e.g. `ollama:nomic-embed-text`). Surfaced by `comemory doctor`; never consumed as a switch. | unset |
| `COMEMORY_RANK_DECAY` | ACT-R decay exponent `d` in `ln(n) − d·ln(days+1)`. Must be ≥ 0. Higher → older memories decay faster. | `0.5` |
| `COMEMORY_RANK_PRIOR_CLAMP` | `"lo,hi"` bounds applied to the activation, feedback, and quality boost multipliers (the fixed `0.2` supersede penalty bypasses the clamp). Both finite; lo > 0, lo ≤ hi. | `0.5,2.0` |
| `COMEMORY_RANK_MMR_LAMBDA` | MMR relevance-vs-diversity trade-off in `[0.0, 1.0]`. `1.0` = pure relevance; `0.0` = pure diversity. | `0.7` |
| `COMEMORY_RANK_NEAR_DUP_HAMMING` | SimHash Hamming radius for near-dup detection (save-time advisory + diversify collapse). Must be ≤ 64. | `8` |
| `COMEMORY_PRUNE_MIN_ACTIVATION` | Activation floor (ACT-R scale) below which a memory is prune-eligible. | `-2.0` |
| `COMEMORY_PRUNE_MIN_FEEDBACK` | Beta-feedback ceiling (range `[0.0, 1.0]`) at or below which a memory is prune-eligible. | `0.25` |
| `COMEMORY_PRUNE_BELOW_QUALITY` | Quality threshold (1..=5); memories at or below this value are prune candidates. | `2` |
| `COMEMORY_PRUNE_SUPERSEDED_GRACE_DAYS` | Grace window (days) before a superseded-and-never-accessed memory becomes prune-eligible. | `7` |

The ranking knobs (`COMEMORY_RANK_*`, `COMEMORY_RETRIEVAL_*`) are explained in
[Measure and tune ranking](guides/ranking-and-eval.md); the `COMEMORY_PRUNE_*`
floors in [Prune, rebuild, and gc](guides/prune-and-gc.md).

## Config-file-only knobs

Set these in `config.toml`; they have **no** environment override.

| Knob | Purpose | Default |
|------|---------|---------|
| `indexing.auto_reindex_threshold_ms` | Debounce (ms) that suppresses spawning bursts of `lazy` auto-reindex processes during rapid successive searches: a new background `index-code` is only spawned if at least this long elapsed since the last trigger. | `200` |
| `tune.rrf_k_grid` / `tune.decay_grid` / `tune.mmr_lambda_grid` / `tune.bm25_grid` | The `[tune]` grid-search axes consumed by `comemory tune` and `comemory bandit`. | — |
| `bandit.enabled` | When `false`, `comemory bandit --apply` refuses; report still works. | `true` |
| `reinforce.search_edit_days` | File overlay for the search→edit lookback (same as `COMEMORY_REINFORCE_SEARCH_EDIT_DAYS`). | `7` |

## Vector dimensions (not configurable)

The memory and code vector dims (`1024` and `768`) are baked into the
`memory_vec` / `code_vec` `vec0` DDL (`src/store/sql/0002_v2_tables.sql`) at
migration time and are **not** env-configurable: a divergent value would
disagree with the vtab and surface as `VecDimMismatch` at first insert. Change
the DDL literal if you need a different dim. See
[Bring your own vectors](guides/byo-vectors.md).

## Pagination envelope

Data-returning commands accept `--limit` / `--offset` (retrieval commands —
`search`, `search-code`, `context` — alias `--limit` to `--k`). With `--json`
they emit a shared `Page` envelope:

```json
{ "items": [ ], "limit": 50, "offset": 0, "total": 123, "has_more": true }
```

- `limit: 0` is the sentinel for "all" (no slicing).
- `total` may be `null` when not counted; for retrieval commands it is the
  in-window ranked count, not a global match count.
- `has_more` is `false` at the end of the window. Ranked retrieval pages stably
  within `COMEMORY_RETRIEVAL_MAX_PAGE_WINDOW` — see the
  [architecture notes](architecture.md) on the retrieval pipeline.
