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
- `index.py` — new standalone function `_enrich_text(memory: Memory) -> str` (not a method — pure function, easily unit-testable without instantiating `MemoryIndex`)
- Called by `_embed_documents` path during `build()` and `upsert()`
- LanceDB stores both `content` (original body, for display in results) and `enriched_content` (metadata + body, for embedding and FTS)
- The FTS index is built on `enriched_content` instead of `content`
- The hybrid search text query targets `enriched_content`
- `_row_to_result` reads `content` (original) for display
- The original markdown body on disk is untouched — enrichment only affects the vector index

**Rationale:** Anthropic's Contextual Retrieval research showed enriched embeddings reduce retrieval failure by 35%. For our small corpus, prepending structured metadata gives the embedding model enough signal to match queries like "cmux" against tags.

---

### Section 2: Cross-Encoder Reranking

After hybrid search retrieves top-20 candidates, a cross-encoder rescores each (query, document) pair.

**Model:** fastembed's `TextCrossEncoder` with `Xenova/ms-marco-MiniLM-L-6-v2` (80MB). No new dependencies — fastembed already ships it.

**Changes:**
- `search.py` — new `_rerank(query: str, results: list[SearchResult], limit: int) -> list[SearchResult]` function
- Reranker is a module-level lazy singleton in `search.py` (not on `MemoryIndex` — keeps separation of concerns: `index.py` handles storage/embedding, `search.py` handles retrieval/ranking)
- Search pipeline becomes: `hybrid search (top 20) → cross-encoder rerank → threshold filter → return top N`

**Verified:** `TextCrossEncoder` confirmed available in fastembed 0.7.4 via `from fastembed.rerank.cross_encoder import TextCrossEncoder`. Supported models: `Xenova/ms-marco-MiniLM-L-6-v2`, `Xenova/ms-marco-MiniLM-L-12-v2`, `jinaai/jina-reranker-v1-tiny-en`, `jinaai/jina-reranker-v1-turbo-en`, `BAAI/bge-reranker-base`, `jinaai/jina-reranker-v2-base-multilingual`.

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
- **Threshold filter (Section 3) operates on `reranker_score` (raw relevance), not `final_score`** — this prevents a relevant-but-old result from being filtered out entirely. The freshness/quality/usage multipliers only affect ranking order, not inclusion.

**Example:** A perfectly relevant 90-day-old bug (`reranker_score=0.85, freshness=0.5, quality_boost=0.84`) gets `final_score=0.32` — if threshold applied to `final_score`, this would be cut at 0.3. Instead, threshold checks `reranker_score=0.85` (passes), then `final_score=0.32` determines its rank position.

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
- `enriched_content` (string) — metadata-enriched text (for embedding + FTS), separate from display `content`

### Modules requiring changes (complete list)

| Module | Changes |
|---|---|
| `index.py` | `_enrich_text()`, `_memory_to_record()` adds `quality` + `enriched_content`, FTS index on `enriched_content`, schema version in `meta.json` |
| `search.py` | Reranker singleton, `_rerank()`, `_apply_thresholds()`, `_freshness_decay()`, `_compute_final_score()`, `_log_search()`, hybrid weight tuning, retrieval count increment |
| `memory.py` | `quality` field on `Memory` (default 3), `parse_memory()` reads `quality` with fallback, `write_memory()` includes `quality` |
| `server.py` | `qwick_memory_feedback` tool, `qwick_memory_save` gains `quality` param, `PROTOCOL` updated with quality rubric + feedback instructions, tiered thresholds recalibrated |
| `cli.py` | `save` command gains `--quality` option (default 3), `search` displays composite score components, `doctor` checks stats file health |
| `config.py` | `get_stats_path()` helper |
| `stats.py` | **New module** — `load_stats()`, `save_stats()`, `increment_retrieval()`, `record_feedback()`, atomic writes via temp-file-then-rename |

### New files

- `stats.py` — Stats I/O for usage tracking (`.stats.json`), atomic writes
- `~/.qwick-memory/.stats.json` — Usage stats sidecar (local, not git-shared)
- `~/.qwick-memory/.search_log.jsonl` — Search interaction log (local, not git-shared)

### New MCP tool

- `qwick_memory_feedback(used_ids, irrelevant_ids)` — Usage feedback

---

## Migration & Org Rollout

**Schema versioning:** `meta.json` gains a `schema_version` field:

```json
{"model": "nomic-ai/nomic-embed-text-v1.5-Q", "schema_version": 2}
```

Current (implicit) version is 1. This release bumps to 2 (enriched embeddings + quality column).

**Automatic migration flow (triggered by `session-start.sh` → `migrate`):**

1. `migrate` reads `meta.json`, compares `schema_version` to code's `SCHEMA_VERSION` constant
2. On mismatch → forces full index rebuild from markdown files on disk
3. Rebuild uses new enrichment format + adds `quality` column (defaults to 3)
4. Writes updated `meta.json` with new `schema_version`

**Backward compatibility across the SidegigLLC org:**

- **Markdown files (git-shared):** New `quality` field in frontmatter is ignored by older plugin versions (python-frontmatter stores unknown fields harmlessly). Older memories without `quality` default to 3 on parse.
- **Vector index (local):** Each dev rebuilds locally. The `schema_version` check ensures automatic rebuild on first session after plugin update.
- **Stats + logs (local):** Created on first use, no migration needed.
- **Cross-encoder model (local):** ~80MB download on first search after update, cached at `~/.cache/fastembed/`. First search is slower; subsequent searches use cache.
- **New MCP tool (`qwick_memory_feedback`):** Discovered automatically by Claude via MCP protocol. No user action needed.

**No breaking changes:** Devs who haven't updated the plugin continue working normally. Devs who update get automatic migration on next session start.

---

## Testing Strategy

**Unit tests:**
- `_enrich_text` — verifies repo, type, tags are prepended; handles empty tags/repo
- `_freshness_decay` — verifies decay values for each memory type at known ages
- `_apply_thresholds` — hard floor filtering, gap detection, edge cases: empty list, single result, all below threshold, monotonically decreasing scores
- `_compute_final_score` — verifies multiplicative combination, default values for missing stats/quality
- `stats.py` — `load_stats` with missing/corrupted file, `save_stats` atomic write, `increment_retrieval`, `record_feedback`
- `parse_memory` — backward compat: memories without `quality` field default to 3

**Integration tests:**
- Search "cmux" returns only the memory that mentions it (or no results if none exist)
- Search with all scores below threshold returns empty list
- Gap detection cuts results at the right position
- Quality and usage boosts affect final ordering
- Enriched content appears in FTS matches (search by tag name that only appears in enrichment)
- Schema version mismatch triggers full rebuild in `migrate`
- Reranker lazy-loads on first search, reuses on subsequent searches

**E2E tests (`scripts/e2e-test.sh`):**
- Save with `--quality` flag, verify frontmatter contains quality field
- Search returns results with composite scores (not raw cosine distances)
- Search for irrelevant query returns 0 results (threshold works)
- Feedback updates `.stats.json`
- Index rebuild after schema version bump
- Save → search → feedback → search again (verify feedback affects ranking)

---

## PROTOCOL Updates (Draft)

### Quality rubric (added to save tool description):

```
Rate quality 1-5 when saving:
- Specificity: names concrete files, functions, versions, error messages? (1=vague, 5=precise)
- Actionability: someone can act on this? (1=trivia, 5=directly useful)
- Context-independence: makes sense in 6 months? (1=needs conversation, 5=self-contained)
Average the three, round to nearest integer. When unsure, default to 3.
```

### Feedback instruction (added after search tool description):

```
After responding to a message where you used qwick_memory_search results:
- Call qwick_memory_feedback with IDs you actually referenced in your response (used_ids)
  and IDs that were irrelevant noise (irrelevant_ids).
- Only call once per response, not per result.
- Skip if you didn't use search in this response.
```

---

## Expected Outcome

For "cmux skill struggles issues problems":
- **Before:** 10 results, scores 0.015-0.032, all irrelevant noise
- **After:** 1 result (the memory mentioning cmux in tags/content), or 0 results if nothing is truly relevant
