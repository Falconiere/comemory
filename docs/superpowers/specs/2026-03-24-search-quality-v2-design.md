# Search Quality v2: Enrichment, Reranking, and Auto-Ranking

**Date:** 2026-03-24
**Status:** Approved
**Author:** falconiere + Claude

## Problem

Search returns garbage results instead of "no results." Example: querying "cmux skill struggles issues problems" returns 10 results with scores 0.015-0.032 — pure noise. The one memory that actually mentions "cmux" is buried at position #6 with a 0.016 score, indistinguishable from irrelevant results.

Root causes:
1. **Only raw content is embedded** — tags, type, and repo metadata are excluded, so searching for "cmux" only matches if the body text contains it
2. **No relevance threshold** — scores of 0.01 are returned instead of "no results found"
3. **No cross-encoder reranking** — cosine similarity produces poorly calibrated scores
4. **No feedback loop** — no mechanism for the system to learn which memories are actually useful
5. **No freshness signal** — a 6-month-old session summary ranks the same as yesterday's decision

## Design

Nine changes organized into three tiers: core retrieval fixes (Sections 1-3), auto-ranking feedback loop (Sections 4-6), and observability (Sections 7-9).

---

### Section 1: Document Enrichment

At index time, construct an enriched text from metadata + content before embedding:

```
[Repository: sidegig-api] [Type: bug] [Tags: cmux, skills, cli]
Skills Directory Structure and Locations

SKILL locations in qwick-apps...
```

**Changes:**
- `index.py` — new `_enrich_text(memory: Memory) -> str` method
- Called by `_embed_documents` path during `build()` and `upsert()`
- The enriched text is stored as `content` in the LanceDB record (so FTS also searches metadata terms)
- The original markdown body on disk is untouched — enrichment only affects the vector index

**Rationale:** Anthropic's Contextual Retrieval research showed enriched embeddings reduce retrieval failure by 35%. For our small corpus, prepending structured metadata gives the embedding model enough signal to match queries like "cmux" against tags.

---

### Section 2: Cross-Encoder Reranking

After hybrid search retrieves top-20 candidates, a cross-encoder rescores each (query, document) pair.

**Model:** fastembed's `TextCrossEncoder` with `Xenova/ms-marco-MiniLM-L-6-v2` (80MB). No new dependencies — fastembed already ships it.

**Changes:**
- `search.py` — new `_rerank(query: str, results: list[SearchResult], limit: int) -> list[SearchResult]` function
- `MemoryIndex` gets a `_reranker: TextCrossEncoder | None` property, lazy-loaded like `_model`
- Search pipeline becomes: `hybrid search (top 20) → cross-encoder rerank → threshold filter → return top N`

**Rationale:** Cross-encoders process query and document together through a transformer, producing calibrated relevance scores. Studies show +10-25% precision improvement over hybrid search alone. At 66 memories, reranking top-20 takes milliseconds.

---

### Section 3: Score Threshold + Gap Detection

After reranking, two filters before returning results:

1. **Hard floor** — Discard results with final score below `MIN_RELEVANCE_SCORE = 0.3`
2. **Gap detector** — If score drops more than `MAX_SCORE_GAP = 0.15` between consecutive results, cut there

Example: scores `[0.82, 0.79, 0.41, 0.38]` → gap between 0.79 and 0.41 → return only first two.

**Changes:**
- `search.py` — new `_apply_thresholds(results, min_score=0.3, max_gap=0.15) -> list[SearchResult]`
- Module-level constants `MIN_RELEVANCE_SCORE` and `MAX_SCORE_GAP` for easy tuning
- Existing tiered formatting thresholds in `server.py` (`HIGH_RELEVANCE_THRESHOLD`, `MODERATE_RELEVANCE_THRESHOLD`) recalibrated for cross-encoder score distributions

**Rationale:** Cross-encoder scores are calibrated enough for absolute thresholds to work. The gap detector catches cases where the first few results are clearly relevant and the rest are noise.

---

### Section 4: Quality Score at Save Time

When Claude saves a memory, it also rates quality (1-5) based on a rubric.

**Rubric (communicated via PROTOCOL):**
- **Specificity** — Does it reference concrete files, functions, versions, error messages?
- **Actionability** — Can someone use this to make a decision or fix a problem?
- **Context-independence** — Will this make sense in 6 months without the original conversation?

Average the three dimensions, round to nearest integer.

**Changes:**
- `qwick_memory_save` — new optional `quality: int` param (1-5, default 3)
- `Memory` dataclass — new `quality: int` field (default 3)
- Stored in frontmatter as `quality: 4` and in LanceDB record
- `PROTOCOL` updated with rubric instructions
- Backward compatible — existing memories without `quality` default to 3

**Search-time boost:** `quality_boost = 0.6 + 0.08 * quality_score` — ranges 0.68 (quality=1) to 1.0 (quality=5). Low-quality memories are deprioritized, not excluded.

---

### Section 5: Usage Feedback Tool

New MCP tool for Claude to report which search results were helpful after responding.

**Interface:**
```python
qwick_memory_feedback(
  used_ids: str,       # comma-separated IDs that were helpful
  irrelevant_ids: str, # comma-separated IDs that were noise
)
```

**Storage:** Sidecar file at `~/.qwick-memory/.stats.json` (not frontmatter, to avoid rewriting markdown on every search):

```json
{
  "32db063a0c31": {"retrieval_count": 12, "usage_count": 8, "last_retrieved": "2026-03-24T..."},
  "ccb023e04f80": {"retrieval_count": 15, "usage_count": 2, "last_retrieved": "2026-03-23T..."}
}
```

**Changes:**
- `server.py` — new `qwick_memory_feedback` tool
- `search.py` — `retrieval_count` auto-incremented when a memory appears in results
- New `stats.py` module — `load_stats()`, `save_stats()`, `increment_retrieval()`, `record_feedback()`
- `PROTOCOL` updated: after responding to a search, call `qwick_memory_feedback`

**Search-time boost:** `usage_boost = 0.8 + 0.2 * (usage_count / max(1, retrieval_count))`. Ranges 0.8 (always irrelevant) to 1.0 (always used). New memories with no history get 0.9 (neutral-positive).

---

### Section 6: Freshness Decay

Time-based decay with type-aware half-lives.

**Formula:** `freshness_decay = exp(-ln(2) / half_life_days * age_days)`

**Half-life table:**

| Memory Type | Half-Life | Rationale |
|---|---|---|
| `convention` | 365 days | Rarely change |
| `preference` | 365 days | Stable user preferences |
| `decision` | 180 days | May be revisited |
| `pattern` | 180 days | Evolve slowly |
| `discovery` | 120 days | May become stale |
| `bug` | 90 days | Bugs get fixed |
| `note` | 60 days | Often ephemeral |
| `session-summary` | 14 days | Only recent sessions matter |

**Changes:**
- `search.py` — `HALF_LIFE_DAYS` dict and `_freshness_decay(created: datetime, memory_type: str) -> float`
- Applied as a multiplier in the combined scoring formula

---

### Section 7: Combined Scoring Formula

All signals combined into a single final score, applied after cross-encoder reranking, before threshold filtering.

**Formula:**
```
final_score = reranker_score * freshness_decay * quality_boost * usage_boost
```

Where:
- `reranker_score` — cross-encoder output (0-1), the primary signal
- `freshness_decay` — `exp(-ln(2) / half_life * age_days)` (0-1)
- `quality_boost` — `0.6 + 0.08 * quality` (0.68-1.0)
- `usage_boost` — `0.8 + 0.2 * usage_ratio` (0.8-1.0), or 0.9 for new memories

**Why multiplicative:** Each factor modifies the base relevance. A completely irrelevant memory (reranker_score=0.05) stays irrelevant regardless of freshness or quality. An old but perfectly relevant memory still surfaces, just slightly deprioritized.

**Changes:**
- `search.py` — new `_compute_final_score(reranker_score, memory_type, created, quality, stats) -> float`
- The threshold filter (Section 3) operates on `final_score`

---

### Section 8: Search Interaction Logging

Append-only log of all search interactions for future analysis and threshold tuning.

**Storage:** `~/.qwick-memory/.search_log.jsonl`, one line per event:

```json
{"timestamp": "2026-03-24T14:30:00+00:00", "type": "search", "query": "cmux skill struggles", "repo_filter": null, "type_filter": null, "tag_filter": null, "results": [{"id": "32db063a0c31", "reranker_score": 0.72, "final_score": 0.65}], "result_count": 1, "filtered_count": 9}
```

Feedback events reference the search:
```json
{"timestamp": "...", "type": "feedback", "used_ids": ["32db063a0c31"], "irrelevant_ids": []}
```

**Changes:**
- `search.py` — `_log_search(query, filters, results, filtered_count)` called at end of `search_memories`, fire-and-forget (logging failure never blocks search)
- Feedback tool in `server.py` also appends to the log
- No log rotation initially — at this scale it will stay small

---

### Section 9: Tuned Hybrid Weights

Explicit hybrid fusion weights instead of LanceDB defaults.

**Change:** `_try_hybrid_search` in `search.py` uses `LinearCombinationReranker(weight=0.5)` — equal vector/FTS weighting instead of default 0.7/0.3.

**Rationale:** Memories contain many specific technical identifiers ("cmux", "CORE-5156", "SuperTokens") that benefit from exact keyword matching. With enriched content (Section 1), the FTS index also covers repo names, tags, and type.

**Full pipeline:**
```
hybrid search (vector 0.5 + FTS 0.5, top 20)
  → cross-encoder rerank
    → combined scoring (freshness × quality × usage)
      → threshold filter (hard floor + gap detection)
        → return results
```

---

## Data Model Changes

### Memory dataclass (`memory.py`)

New field:
- `quality: int` — 1-5 quality rating (default 3)

### Frontmatter schema

```yaml
---
id: a1b2c3d4e5f6
repo: [sidegig-api]
type: decision
tags: [database, postgres]
author: falconiere
created: 2026-03-24T14:30:00+00:00
quality: 4                    # NEW
content_hash: a1b2c3d4e5f6
---
```

### LanceDB record

New columns:
- `quality` (int) — quality score
- `enriched_content` (string) — metadata-enriched text (for FTS), separate from display `content`

### New files

- `stats.py` — Stats I/O for usage tracking (`.stats.json`)
- `~/.qwick-memory/.stats.json` — Usage stats sidecar
- `~/.qwick-memory/.search_log.jsonl` — Search interaction log

### New MCP tool

- `qwick_memory_feedback(used_ids, irrelevant_ids)` — Usage feedback

---

## Migration

- Existing memories without `quality` field default to 3
- Index rebuild required (enriched embeddings + new `quality` column)
- The `migrate` command detects the schema change and triggers rebuild
- Stats file created on first feedback call

---

## Testing Strategy

- Unit tests for `_enrich_text`, `_freshness_decay`, `_apply_thresholds`, `_compute_final_score`
- Integration test: search "cmux" returns only the memory that mentions it (or no results if none exist)
- Integration test: search with all scores below threshold returns empty list
- Integration test: gap detection cuts results correctly
- Integration test: quality and usage boosts affect final ordering
- E2E test: save → search → feedback → verify stats updated
- E2E test: enriched content appears in FTS matches

---

## Expected Outcome

For "cmux skill struggles issues problems":
- **Before:** 10 results, scores 0.015-0.032, all irrelevant noise
- **After:** 1 result (the memory mentioning cmux in tags/content), or 0 results if nothing is truly relevant
