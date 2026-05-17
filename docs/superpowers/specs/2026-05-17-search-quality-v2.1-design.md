# Search Quality v2.1 — Correctness + Performance + Quality

**Date:** 2026-05-17
**Status:** Approved, pending implementation plan
**Predecessor:** `2026-03-24-search-quality-v2-design.md` (shipped as v0.2.0)

## Motivation

Post-ship audit of the v2 search pipeline (`src/qwick_memory/search.py`,
`index.py`, `stats.py`) surfaced 16 issues across correctness, performance, and
quality. This spec bundles all three groups into a single follow-up release.
Pipeline shape is unchanged; the work is surgical fixes inside existing stages.

## Issues addressed

### Correctness (A)

- **A.1** LIKE-clause sanitizer escapes `"` and `%` but leaves the `_` wildcard
  unescaped, so a `repo` value containing `_` matches unintended rows.
- **A.2** `MIN_RELEVANCE_SCORE = 0.3` is hardcoded and uncalibrated against the
  sigmoid-normalized cross-encoder logits. Valid queries can be silently
  zeroed out.
- **A.3** `_compute_final_score` returns `usage_boost = 0.9` for memories with
  no stats and `0.8` for memories with stats but zero usage. New (unseen)
  memories are penalized less than retrieved-but-never-used ones — the
  direction is right but the magnitudes are flipped: an unseen memory should
  be **neutral**, not less-bad than a known dud.
- **A.4** `session-summary` half-life of 14 days drops scores by ~75% within a
  month; combined with the threshold filter, recent summaries can be dropped
  entirely.
- **A.5** `record_feedback` accepts `irrelevant_ids` but only increments
  `usage_count` for `used_ids`. Negative feedback is collected and discarded.

### Performance (B)

- **B.6** `_log_search` performs a synchronous JSONL `open`/`write` on every
  search return path. Slow disks tax latency on the hot path.
- **B.7** `load_stats()` reads and JSON-parses the full stats file on every
  search.
- **B.8** `increment_retrieval` does a read-modify-write of the full stats
  file per search, racing with feedback writes.
- **B.9** `_get_reranker()` lazy-loads the ONNX cross-encoder on the first
  search of each MCP server lifetime, producing a latency cliff for the first
  user-facing query.

### Quality (C)

- **C.11** Hybrid `LinearCombinationReranker(weight=0.5)` is hardcoded and not
  configurable.
- **C.12** `_rerank` feeds `r.content` to the cross-encoder, but the indexed
  documents are `enriched_content` (with `[Repository: ...] [Type: ...]
  [Tags: ...]` prefix). The reranker sees less context than vector + FTS.
- **C.13** After `_apply_thresholds`, the result count can fall below `limit`
  even when additional valid documents sit just under the gap cutoff. The
  pipeline returns fewer results than requested without a backfill step.
- **C.14** No dedup: near-identical memories (same fix saved twice, mirrored
  conventions) both rank high and consume slots.
- **C.15** Reranker model, thresholds, and hybrid weights have no env
  overrides — tuning requires code edits.

Issue #16 (broad `except Exception` swallows) is folded into the relevant
component changes below rather than tracked separately.

## Architecture

The pipeline stages are unchanged:

```
vector / hybrid search
        ↓
rerank (cross-encoder)
        ↓
threshold filter
        ↓
combined scoring (freshness × quality × usage)
        ↓
top-K
```

Three new internal steps are inserted:

```
        rerank (now on enriched_content)
                ↓
        threshold filter
                ↓
   ▶▶ backfill (if filtered < limit, restore from candidates)
                ↓
   ▶▶ dedup (cosine on stored vectors, threshold 0.92)
                ↓
        combined scoring
                ↓
   ▶▶ async log + append-only stats event
```

No new dependencies. No LanceDB schema change. `SCHEMA_VERSION` stays `2`.

## Module changes

| Module | Change |
|--------|--------|
| `search.py` | A.1 LIKE escape helper; A.3 usage boost formula; A.5 read `irrelevance_count`; C.12 rerank on `enriched_content`; C.13 backfill; C.14 dedup; C.15 read config constants |
| `stats.py` | A.5 `record_feedback` increments `irrelevance_count` for `irrelevant_ids`; deprecates per-call read-modify-write — delegates to `stats_cache` |
| `stats_cache.py` (NEW) | B.7 cached stats with mtime invalidation; B.8 append-only event log + compactor |
| `config.py` | C.15 env vars: `QWICK_MEMORY_RERANKER_MODEL`, `QWICK_MEMORY_MIN_RELEVANCE`, `QWICK_MEMORY_MAX_GAP`, `QWICK_MEMORY_HYBRID_WEIGHT` |
| `server.py` | B.9 eager reranker preload before stdio loop |
| `cli.py` | A.2 `qwick-memory calibrate` subcommand (reads search log, prints threshold recommendations from feedback distribution) |

## Component details

### A.1 — LIKE wildcard escape

Add to `search.py`:

```python
def _escape_like(s: str) -> str:
    """Escape LIKE wildcards. Backslash, percent, underscore."""
    return s.replace("\\", "\\\\").replace("%", "\\%").replace("_", "\\_")
```

Apply to `repo` and `tag` filters. Quote-escaping for `"` stays as today.
LanceDB's SQL dialect supports `ESCAPE` clause; the LIKE clauses become
`repo LIKE "%escaped%" ESCAPE '\\'`.

### A.2 — Threshold calibration

Default `MIN_RELEVANCE_SCORE` stays `0.3` (no behavior break for upgraders).
Resolved at module load from `config.MIN_RELEVANCE_SCORE`, which reads
`QWICK_MEMORY_MIN_RELEVANCE` env var, falls back to `0.3`.

New CLI: `qwick-memory calibrate`

- Reads `search.events.jsonl` (search log).
- Joins with `stats.json` to find memories with feedback.
- For each used vs irrelevant memory, prints the `reranker_score`
  distribution (P10, P25, P50, P75, P90).
- Recommends a threshold at the value that separates used from irrelevant
  with the best F1 (or, if no negative feedback exists, the P10 of used).
- Output is a recommendation only — user copies the value into their env.

### A.3 — Usage boost asymmetry

```python
def _usage_boost(stats: dict | None) -> float:
    if stats is None:
        return 1.0  # unseen — neutral
    retrieval_count = stats.get("retrieval_count", 0)
    if retrieval_count == 0:
        return 1.0
    usage_count = stats.get("usage_count", 0)
    irrelevance_count = stats.get("irrelevance_count", 0)
    net = max(0, usage_count - irrelevance_count)
    return 0.8 + 0.2 * (net / retrieval_count)
```

Range: `0.8` (retrieved 10x, never used or fully marked irrelevant) to `1.0`
(used every time). Unseen memories no longer worse than known duds.

### A.4 — session-summary half-life

`HALF_LIFE_DAYS["session-summary"]` from `14` to `30`. The intent (summaries
decay faster than decisions) is preserved; the magnitude was too aggressive.
No callers depend on the specific value — `qwick_memory_context` uses a
separate token-budgeted retrieval path.

### A.5 — Irrelevant feedback

In `stats.py::record_feedback`:

```python
for mid in irrelevant_ids:
    if mid not in stats:
        stats[mid] = {"retrieval_count": 1, "usage_count": 0,
                      "irrelevance_count": 0, "last_retrieved": ""}
    stats[mid]["irrelevance_count"] = stats[mid].get("irrelevance_count", 0) + 1
```

`_usage_boost` consumes the new field (see A.3).

### B.6 — Async search log

```python
_log_executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix="qm-log")

def _log_search(...):
    _log_executor.submit(_log_search_sync, ...)
```

Single worker preserves write order. On interpreter exit, in-flight log
lines may be lost — acceptable for analytics. No `atexit` join needed.

### B.7 / B.8 — `stats_cache.py`

```python
# Module-level cache
_cache: dict[str, dict] | None = None
_cache_mtime: float = 0.0

def get_stats() -> dict[str, dict]:
    """Cached stats; reloads on mtime change."""

def append_event(event: dict) -> None:
    """Append one line to stats.events.jsonl. Fire-and-forget."""

def compact() -> None:
    """Fold events into stats.json. Called on index rebuild or every N events."""
```

`increment_retrieval` and `record_feedback` switch from
read-modify-write of `stats.json` to `append_event(...)`. Compaction runs:

- On `qwick-memory index` (CLI explicit).
- Automatically when `stats.events.jsonl` size exceeds 1 MB
  (`append_event` checks file size after write; if over the threshold,
  it submits compaction to a thread).

Race safety: the compactor reads events under an advisory lock
(`fcntl.flock`), folds into a temp file, renames atomically over
`stats.json`, then truncates the events file.

### B.9 — Eager reranker preload

`server.py::main()` adds:

```python
from qwick_memory.search import _get_reranker
_get_reranker()  # pay cold start before stdio loop
```

Adds ~1s to MCP server start; eliminates per-session first-query cliff.

### C.11 — Hybrid weight env

`config.HYBRID_WEIGHT = float(os.environ.get("QWICK_MEMORY_HYBRID_WEIGHT", "0.5"))`.
`_try_hybrid_search` reads from config.

### C.12 — Rerank on enriched_content

```python
documents = [r.enriched_content or r.content for r in results]
```

Field is already populated by `_row_to_result`.

### C.13 — Threshold backfill

After `_apply_thresholds(results)`:

```python
if len(filtered) < limit:
    remaining = [r for r in results_pre_threshold if r not in filtered
                 and r.reranker_score >= min_score]
    filtered.extend(remaining[: limit - len(filtered)])
```

`remaining` is bounded by `min_score` so the floor is still respected; only
the gap rule is relaxed when results are scarce.

### C.14 — Dedup

`SearchResult` gains an optional `vector: list[float] | None = None`.
`_row_to_result` extracts the `vector` column from LanceDB rows.

```python
def _dedup(results: list[SearchResult], threshold: float = 0.92) -> list[SearchResult]:
    kept: list[SearchResult] = []
    for r in results:
        if r.vector is None:
            kept.append(r)
            continue
        if any(_cosine(r.vector, k.vector) >= threshold
               for k in kept if k.vector is not None):
            continue
        kept.append(r)
    return kept
```

Greedy: keep first, drop later near-duplicates. Threshold `0.92` is high
enough to require near-identical content (validated separately during
implementation by sampling pairs from the corpus).

### C.15 — Env overrides

`config.py` exports module-level constants resolved once at import:

- `RERANKER_MODEL` — defaults to `Xenova/ms-marco-MiniLM-L-6-v2`.
- `MIN_RELEVANCE_SCORE` — defaults to `0.3`.
- `MAX_SCORE_GAP` — defaults to `0.15`.
- `HYBRID_WEIGHT` — defaults to `0.5`.

`search.py` imports from `config`, removes its module-level constants.

## Error handling

Replace broad `except Exception: logger.debug(...)` in `_log_search`,
`increment_retrieval`, and `_get_reranker` cold paths with:

```python
except (OSError, json.JSONDecodeError) as e:
    logger.warning("...", exc_info=e)
```

`contextlib.suppress(Exception)` is retained only around best-effort
`table.optimize()` and `table.create_fts_index(...)` in `index.py`, which
are non-essential.

## Testing

### New unit tests

| Test | Validates |
|------|-----------|
| `test_search.py::test_like_wildcard_escape` | Repo filter with `_` and `%` matches only exact-substring rows |
| `test_search.py::test_usage_boost_unseen_neutral` | Unseen memories get boost `1.0` |
| `test_search.py::test_usage_boost_irrelevant_lowers` | High `irrelevance_count` drops boost toward `0.8` |
| `test_search.py::test_rerank_uses_enriched_content` | Reranker input includes `[Repository: ...]` prefix |
| `test_search.py::test_threshold_backfill_fills_to_limit` | When gap cuts results below `limit`, backfill restores up to `limit` while respecting `min_score` |
| `test_search.py::test_dedup_drops_near_duplicates` | Two memories with cosine ≥ 0.92 → only one kept |
| `test_search.py::test_env_overrides_applied` | Setting `QWICK_MEMORY_MIN_RELEVANCE=0.5` changes filter cutoff |
| `test_stats_cache.py::test_mtime_invalidation` | Cache reloads after external file change |
| `test_stats_cache.py::test_append_event_concurrent` | Two threads append events without corrupting JSONL |
| `test_stats_cache.py::test_compact_folds_events` | Events file folded into `stats.json` after compact |
| `test_stats.py::test_record_feedback_irrelevant` | `irrelevance_count` increments for `irrelevant_ids` |
| `test_cli.py::test_calibrate_outputs_recommendation` | `qwick-memory calibrate` prints percentiles + recommended threshold |

### Updated tests

`test_search.py` assertions that compare specific final-score values are
re-baselined for the new `_usage_boost` formula. The relative ordering
contract is preserved; only absolute magnitudes shift.

### E2E (`scripts/e2e-test.sh`)

Add checks:

- `qwick-memory calibrate` exits 0 and prints a threshold value.
- `QWICK_MEMORY_MIN_RELEVANCE=0.99 qwick-memory search "x"` returns zero
  results (env override threaded through).
- After `qwick-memory feedback --irrelevant <id>`, the memory's score on the
  next identical search is lower than before.

## Migration

- `stats.events.jsonl` created on first event — no migration script needed.
- `irrelevance_count` defaults to `0` when missing from existing stats.
- Defaults for `MIN_RELEVANCE_SCORE` and `HYBRID_WEIGHT` unchanged, so
  upgraders see no behavior change unless they set the new env vars.
- `SCHEMA_VERSION` stays `2`. No vector index rebuild required.
- `_rerank` change (C.12) does not require reindexing; `enriched_content`
  is already in the index since v2.

## Out of scope

- BM25/FTS internal tuning beyond the hybrid-weight knob.
- Query expansion or spell correction.
- Replacing the JSON stats file with SQLite (considered for B.8 and rejected
  — append-only JSONL plus a compactor is simpler and adds no dependencies).
- Reranker model swap. The env var lets a user swap, but no default change
  ships in this release.

## Rollout

- Single PR. No flag gating.
- Version bump to `0.2.1` (correctness + perf + quality bundle, no breaking
  changes).
- `CHANGELOG.md` entry summarizing user-visible effects:
  unseen memories no longer down-ranked, irrelevant feedback now applied,
  faster first query in MCP, configurable thresholds.
