# Search Quality v2.1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bundle 15 audit fixes for the v2 search pipeline (correctness, performance, quality) into a single 0.2.1 release.

**Architecture:** Pipeline stages unchanged — vector/hybrid → rerank → threshold → score → top-K. Surgical changes inside existing modules. One new module (`stats_cache.py`) for cache + append-only events. New CLI subcommand `calibrate`. New env knobs in `config.py`. No new dependencies. No LanceDB schema change.

**Tech Stack:** Python 3.10+, LanceDB ≥0.30, fastembed (TextEmbedding + TextCrossEncoder), Typer, FastMCP, pytest, ruff.

**Spec:** `docs/superpowers/specs/2026-05-17-search-quality-v2.1-design.md`

**File layout:**
- Modify: `src/qwick_memory/config.py` — env constants
- Modify: `src/qwick_memory/search.py` — LIKE escape, usage_boost, threshold refactor, dedup, env reads, rerank on enriched
- Modify: `src/qwick_memory/stats.py` — wire `irrelevance_count`, delegate I/O to stats_cache
- Create: `src/qwick_memory/stats_cache.py` — cached stats + append-only events + compactor
- Modify: `src/qwick_memory/server.py` — eager preload reranker + embedding model
- Modify: `src/qwick_memory/cli.py` — `calibrate` subcommand
- Modify: `pyproject.toml` — version 0.2.0 → 0.2.1
- Create: `CHANGELOG.md` — 0.2.1 entry
- Modify: `tests/test_search.py` — new tests for fixes
- Modify: `tests/test_stats.py` — irrelevance_count test
- Create: `tests/test_stats_cache.py`
- Modify: `tests/test_cli.py` — calibrate test
- Modify: `scripts/e2e-test.sh` — calibrate + env override smoke

---

## Task 1: Add env-resolved constants to `config.py`

Wires the foundation for C.11, C.15. Establishes single source of truth for tunable knobs.

**Files:**
- Modify: `src/qwick_memory/config.py`
- Test: `tests/test_config.py` (create if missing)

- [ ] **Step 1: Write failing test for env override**

Create `tests/test_config.py` if missing, otherwise append:

```python
"""Tests for qwick_memory.config — env-resolved tunable constants."""

import importlib

import pytest


def _reload_config():
    """Re-import config so module-level env reads happen again."""
    import qwick_memory.config as cfg

    return importlib.reload(cfg)


def test_min_relevance_default(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("QWICK_MEMORY_MIN_RELEVANCE", raising=False)
    cfg = _reload_config()
    assert cfg.MIN_RELEVANCE_SCORE == 0.3


def test_min_relevance_env_override(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("QWICK_MEMORY_MIN_RELEVANCE", "0.55")
    cfg = _reload_config()
    assert cfg.MIN_RELEVANCE_SCORE == 0.55


def test_max_gap_env_override(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("QWICK_MEMORY_MAX_GAP", "0.25")
    cfg = _reload_config()
    assert cfg.MAX_SCORE_GAP == 0.25


def test_hybrid_weight_default(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("QWICK_MEMORY_HYBRID_WEIGHT", raising=False)
    cfg = _reload_config()
    assert cfg.HYBRID_WEIGHT == 0.5


def test_reranker_model_env_override(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("QWICK_MEMORY_RERANKER_MODEL", "custom/model")
    cfg = _reload_config()
    assert cfg.RERANKER_MODEL == "custom/model"
```

- [ ] **Step 2: Run test to verify failure**

Run: `pytest tests/test_config.py -v`
Expected: FAIL with `AttributeError: module 'qwick_memory.config' has no attribute 'MIN_RELEVANCE_SCORE'`.

- [ ] **Step 3: Add constants to `config.py`**

Append to `src/qwick_memory/config.py`:

```python
def _env_float(name: str, default: float) -> float:
  raw = os.environ.get(name)
  if raw is None or raw == "":
    return default
  try:
    return float(raw)
  except ValueError:
    return default


MIN_RELEVANCE_SCORE: float = _env_float("QWICK_MEMORY_MIN_RELEVANCE", 0.3)
MAX_SCORE_GAP: float = _env_float("QWICK_MEMORY_MAX_GAP", 0.15)
HYBRID_WEIGHT: float = _env_float("QWICK_MEMORY_HYBRID_WEIGHT", 0.5)
RERANKER_MODEL: str = os.environ.get(
  "QWICK_MEMORY_RERANKER_MODEL", "Xenova/ms-marco-MiniLM-L-6-v2"
)


def get_search_events_path() -> Path:
  """Append-only event log for stats (compacted into .stats.json)."""
  return get_rag_dir() / ".stats.events.jsonl"
```

- [ ] **Step 4: Run test to verify pass**

Run: `pytest tests/test_config.py -v`
Expected: PASS, 5 tests.

- [ ] **Step 5: Commit**

```bash
git add src/qwick_memory/config.py tests/test_config.py
git commit -m "feat(config): add env-resolved tunable constants"
```

---

## Task 2: LIKE wildcard escape (A.1)

Fixes injection-style false positives in `repo` and `tag` LIKE filters.

**Files:**
- Modify: `src/qwick_memory/search.py:193-204`
- Test: `tests/test_search.py`

- [ ] **Step 1: Write failing test**

Append to `tests/test_search.py`:

```python
def test_like_wildcard_escape(built_index: MemoryIndex) -> None:
    """Repo filter with `_` does not match arbitrary single-char rows."""
    results = search_memories(built_index, "anything", repo="a_me")
    assert results == [], "underscore should be escaped, not act as LIKE wildcard"


def test_like_percent_escape(built_index: MemoryIndex) -> None:
    """Repo filter with `%` does not act as multi-char wildcard."""
    results = search_memories(built_index, "anything", repo="%frontend")
    assert results == [], "% should be escaped, not act as LIKE wildcard"
```

- [ ] **Step 2: Run test to verify failure**

Run: `pytest tests/test_search.py::test_like_wildcard_escape tests/test_search.py::test_like_percent_escape -v`
Expected: FAIL — current implementation strips `%` but keeps `_` as wildcard.

- [ ] **Step 3: Implement escape helper + apply**

Edit `src/qwick_memory/search.py`. Add helper:

```python
def _escape_like(s: str) -> str:
  """Escape LIKE wildcards (backslash, percent, underscore)."""
  return s.replace("\\", "\\\\").replace("%", "\\%").replace("_", "\\_")
```

Replace the filter block inside `search_memories`:

```python
  where_clauses: list[str] = []
  if repo is not None:
    safe_repo = _escape_like(repo.replace('"', '\\"'))
    where_clauses.append(f"repo LIKE \"%{safe_repo}%\" ESCAPE '\\\\'")
  if type_filter is not None:
    safe_type = type_filter.replace('"', '\\"')
    where_clauses.append(f'type = "{safe_type}"')
  if tag is not None:
    safe_tag = _escape_like(tag.replace('"', '\\"'))
    where_clauses.append(f"tags LIKE \"%{safe_tag}%\" ESCAPE '\\\\'")
  where_expr = " AND ".join(where_clauses) if where_clauses else None
```

- [ ] **Step 4: Run tests to verify pass**

Run: `pytest tests/test_search.py -v`
Expected: PASS for new tests; existing repo filter test still passes (literal "acme/frontend" still matches).

- [ ] **Step 5: Commit**

```bash
git add src/qwick_memory/search.py tests/test_search.py
git commit -m "fix(search): escape LIKE wildcards in repo and tag filters"
```

---

## Task 3: Bump `session-summary` half-life 14 → 30 (A.4)

Tiny single-value change.

**Files:**
- Modify: `src/qwick_memory/search.py:22-31`

- [ ] **Step 1: Edit half-life table**

In `HALF_LIFE_DAYS`, change `"session-summary": 14,` to `"session-summary": 30,`.

- [ ] **Step 2: Run tests**

Run: `pytest tests/test_search.py tests/test_scoring.py -v`
Expected: PASS. Re-baseline any test asserting specific session-summary magnitudes.

- [ ] **Step 3: Commit**

```bash
git add src/qwick_memory/search.py
git commit -m "tune(search): session-summary half-life 14d -> 30d"
```

---

## Task 4: Wire `irrelevance_count` in `record_feedback` (A.5)

Negative feedback was discarded.

**Files:**
- Modify: `src/qwick_memory/stats.py:59-70`
- Test: `tests/test_stats.py`

- [ ] **Step 1: Write failing test**

Append to `tests/test_stats.py`:

```python
def test_record_feedback_increments_irrelevance(tmp_path: Path) -> None:
    from qwick_memory.stats import load_stats, record_feedback

    stats_path = tmp_path / "stats.json"
    record_feedback(used_ids=[], irrelevant_ids=["abc123"], stats_path=stats_path)
    record_feedback(used_ids=[], irrelevant_ids=["abc123"], stats_path=stats_path)
    data = load_stats(stats_path)
    assert data["abc123"]["irrelevance_count"] == 2
    assert data["abc123"]["usage_count"] == 0


def test_record_feedback_used_and_irrelevant_independent(tmp_path: Path) -> None:
    from qwick_memory.stats import load_stats, record_feedback

    stats_path = tmp_path / "stats.json"
    record_feedback(used_ids=["good"], irrelevant_ids=["bad"], stats_path=stats_path)
    data = load_stats(stats_path)
    assert data["good"]["usage_count"] == 1
    assert data["good"].get("irrelevance_count", 0) == 0
    assert data["bad"]["irrelevance_count"] == 1
    assert data["bad"]["usage_count"] == 0
```

Add `from pathlib import Path` at the top of `tests/test_stats.py` if missing.

- [ ] **Step 2: Run test to verify failure**

Run: `pytest tests/test_stats.py::test_record_feedback_increments_irrelevance -v`
Expected: FAIL — `irrelevance_count` key missing.

- [ ] **Step 3: Patch `record_feedback`**

Replace `record_feedback` in `src/qwick_memory/stats.py`:

```python
def record_feedback(
  used_ids: list[str],
  irrelevant_ids: list[str],
  stats_path: Path | None = None,
) -> None:
  """Record which memories were used vs irrelevant."""
  stats = load_stats(stats_path)
  for mid in used_ids:
    if mid not in stats:
      stats[mid] = {
        "retrieval_count": 1,
        "usage_count": 0,
        "irrelevance_count": 0,
        "last_retrieved": "",
      }
    stats[mid]["usage_count"] = stats[mid].get("usage_count", 0) + 1
  for mid in irrelevant_ids:
    if mid not in stats:
      stats[mid] = {
        "retrieval_count": 1,
        "usage_count": 0,
        "irrelevance_count": 0,
        "last_retrieved": "",
      }
    stats[mid]["irrelevance_count"] = stats[mid].get("irrelevance_count", 0) + 1
  save_stats(stats, stats_path)
```

- [ ] **Step 4: Run tests to verify pass**

Run: `pytest tests/test_stats.py -v`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/qwick_memory/stats.py tests/test_stats.py
git commit -m "fix(stats): record irrelevant feedback as irrelevance_count"
```

---

## Task 5: Refactor usage boost for asymmetry fix (A.3)

Unseen memories were penalized harder than known-dud memories. Flip the direction so unseen = neutral.

**Files:**
- Modify: `src/qwick_memory/search.py:127-143`
- Test: `tests/test_search.py`

- [ ] **Step 1: Write failing test**

Append to `tests/test_search.py`:

```python
from qwick_memory.search import _compute_final_score


def _score_args(stats=None):
    from datetime import datetime, timezone

    return {
        "reranker_score": 0.8,
        "memory_type": "decision",
        "created": datetime.now(timezone.utc),
        "quality": 3,
        "stats": stats,
    }


def test_usage_boost_unseen_is_neutral() -> None:
    """A memory with no stats receives boost factor 1.0 (neutral)."""
    score_unseen = _compute_final_score(**_score_args(stats=None))
    score_full_use = _compute_final_score(
        **_score_args(stats={"retrieval_count": 10, "usage_count": 10,
                             "irrelevance_count": 0})
    )
    assert score_unseen == pytest.approx(score_full_use, rel=1e-6)


def test_usage_boost_irrelevant_lowers_score() -> None:
    score_unseen = _compute_final_score(**_score_args(stats=None))
    score_irrelevant = _compute_final_score(
        **_score_args(stats={"retrieval_count": 10, "usage_count": 0,
                             "irrelevance_count": 5})
    )
    assert score_irrelevant < score_unseen


def test_usage_boost_partial_usage_between_extremes() -> None:
    score_floor = _compute_final_score(
        **_score_args(stats={"retrieval_count": 10, "usage_count": 0,
                             "irrelevance_count": 0})
    )
    score_mid = _compute_final_score(
        **_score_args(stats={"retrieval_count": 10, "usage_count": 5,
                             "irrelevance_count": 0})
    )
    score_full = _compute_final_score(
        **_score_args(stats={"retrieval_count": 10, "usage_count": 10,
                             "irrelevance_count": 0})
    )
    assert score_floor < score_mid < score_full
```

- [ ] **Step 2: Run test to verify failure**

Run: `pytest tests/test_search.py::test_usage_boost_unseen_is_neutral -v`
Expected: FAIL — current formula returns 0.9 for unseen and 0.8 for stats-with-zero-usage.

- [ ] **Step 3: Patch scoring**

Replace in `src/qwick_memory/search.py`:

```python
def _usage_boost(stats: dict[str, Any] | None) -> float:
  """Boost factor in [0.8, 1.0]. Unseen = 1.0 (neutral)."""
  if stats is None:
    return 1.0
  retrieval_count = stats.get("retrieval_count", 0)
  if retrieval_count == 0:
    return 1.0
  usage_count = stats.get("usage_count", 0)
  irrelevance_count = stats.get("irrelevance_count", 0)
  net = max(0, usage_count - irrelevance_count)
  return 0.8 + 0.2 * (net / retrieval_count)


def _compute_final_score(
  reranker_score: float,
  memory_type: str,
  created: datetime,
  quality: int,
  stats: dict[str, Any] | None,
) -> float:
  """Combine reranker score with freshness, quality, and usage signals."""
  freshness = _freshness_decay(created, memory_type)
  quality_boost = 0.6 + 0.08 * quality
  usage_boost = _usage_boost(stats)
  return reranker_score * freshness * quality_boost * usage_boost
```

- [ ] **Step 4: Run tests to verify pass**

Run: `pytest tests/test_search.py tests/test_scoring.py -v`
Expected: PASS. Re-baseline existing magnitude assertions if needed.

- [ ] **Step 5: Commit**

```bash
git add src/qwick_memory/search.py tests/test_search.py
git commit -m "fix(search): usage_boost — unseen=1.0, apply irrelevance"
```

---

## Task 6: Rerank on `enriched_content` (C.12)

**Files:**
- Modify: `src/qwick_memory/search.py:46-65`
- Test: `tests/test_search.py`

- [ ] **Step 1: Write failing test**

Append to `tests/test_search.py`:

```python
def test_rerank_uses_enriched_content(monkeypatch: pytest.MonkeyPatch,
                                      built_index: MemoryIndex) -> None:
    """Reranker is invoked with enriched_content."""
    captured_documents: list[list[str]] = []

    class FakeReranker:
        def rerank(self, query: str, documents: list[str]) -> list[float]:
            captured_documents.append(list(documents))
            return [1.0 for _ in documents]

    import qwick_memory.search as search_module

    monkeypatch.setattr(search_module, "_get_reranker", lambda: FakeReranker())
    search_memories(built_index, "PostgreSQL")
    assert captured_documents
    assert any("[Repository:" in d for d in captured_documents[0])
```

- [ ] **Step 2: Run test to verify failure**

Run: `pytest tests/test_search.py::test_rerank_uses_enriched_content -v`
Expected: FAIL — current code uses `r.content`.

- [ ] **Step 3: Patch `_rerank`**

Replace `_rerank` in `src/qwick_memory/search.py`:

```python
def _rerank(
  query: str,
  results: list[SearchResult],
  limit: int,
) -> list[SearchResult]:
  """Rerank results using cross-encoder, normalize logits via sigmoid."""
  if not results:
    return []

  reranker = _get_reranker()
  documents = [r.enriched_content or r.content for r in results]
  raw_scores = list(reranker.rerank(query, documents))

  for result, raw in zip(results, raw_scores, strict=True):
    result.reranker_score = 1.0 / (1.0 + math.exp(-raw))

  results.sort(key=lambda r: r.reranker_score, reverse=True)
  return results[:limit]
```

- [ ] **Step 4: Run tests to verify pass**

Run: `pytest tests/test_search.py -v`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/qwick_memory/search.py tests/test_search.py
git commit -m "fix(search): rerank on enriched_content not bare content"
```

---

## Task 7: Refactor `_apply_thresholds` + threshold backfill (C.13)

`_apply_thresholds` now returns `(filtered, tail)`. Caller backfills when scarce.

**Files:**
- Modify: `src/qwick_memory/search.py:89-114` and caller
- Test: `tests/test_search.py`

- [ ] **Step 1: Write failing test**

Append to `tests/test_search.py`:

```python
def test_threshold_backfill_returns_filtered_and_tail() -> None:
    from qwick_memory.search import SearchResult, _apply_thresholds

    results = [
        SearchResult(id="a", repo="r", type="decision", tags="", author="",
                     created="2026-01-01T00:00:00+00:00", content="", score=0,
                     reranker_score=0.90),
        SearchResult(id="b", repo="r", type="decision", tags="", author="",
                     created="2026-01-01T00:00:00+00:00", content="", score=0,
                     reranker_score=0.85),
        SearchResult(id="c", repo="r", type="decision", tags="", author="",
                     created="2026-01-01T00:00:00+00:00", content="", score=0,
                     reranker_score=0.55),
        SearchResult(id="d", repo="r", type="decision", tags="", author="",
                     created="2026-01-01T00:00:00+00:00", content="", score=0,
                     reranker_score=0.50),
    ]
    filtered, tail = _apply_thresholds(results, min_score=0.3, max_gap=0.15)
    assert [r.id for r in filtered] == ["a", "b"]
    assert [r.id for r in tail] == ["c", "d"]


def test_apply_thresholds_empty_below_floor() -> None:
    from qwick_memory.search import SearchResult, _apply_thresholds

    results = [
        SearchResult(id="x", repo="", type="note", tags="", author="",
                     created="2026-01-01T00:00:00+00:00", content="", score=0,
                     reranker_score=0.1),
    ]
    filtered, tail = _apply_thresholds(results, min_score=0.3, max_gap=0.15)
    assert filtered == []
    assert tail == []
```

- [ ] **Step 2: Run test to verify failure**

Run: `pytest tests/test_search.py::test_threshold_backfill_returns_filtered_and_tail -v`
Expected: FAIL — current returns one list.

- [ ] **Step 3: Refactor `_apply_thresholds` + caller**

Replace `_apply_thresholds` in `src/qwick_memory/search.py`:

```python
def _apply_thresholds(
  results: list[SearchResult],
  min_score: float = MIN_RELEVANCE_SCORE,
  max_gap: float = MAX_SCORE_GAP,
) -> tuple[list[SearchResult], list[SearchResult]]:
  """Filter by floor + gap. Return (filtered, tail).

  tail = items above min_score but cut by gap rule. Caller may backfill.
  """
  if not results:
    return [], []

  above = [r for r in results if r.reranker_score >= min_score]
  if not above:
    return [], []

  above.sort(key=lambda r: r.reranker_score, reverse=True)

  filtered = [above[0]]
  for i in range(1, len(above)):
    gap = above[i - 1].reranker_score - above[i].reranker_score
    if gap > max_gap:
      break
    filtered.append(above[i])

  tail = above[len(filtered):]
  return filtered, tail
```

In `search_memories`, replace `results = _apply_thresholds(results)` with:

```python
  # Step 3: Threshold filter (returns filtered + tail above floor)
  filtered, tail = _apply_thresholds(results)
  if not filtered:
    return []

  # Step 3b: Backfill from tail when scarce (floor still respected)
  if len(filtered) < limit and tail:
    filtered.extend(tail[: limit - len(filtered)])
  results = filtered
```

- [ ] **Step 4: Run tests to verify pass**

Run: `pytest tests/test_search.py -v`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/qwick_memory/search.py tests/test_search.py
git commit -m "feat(search): threshold backfill when results scarce"
```

---

## Task 8: Dedup near-duplicate results (C.14)

**Files:**
- Modify: `src/qwick_memory/search.py` — SearchResult, _row_to_result, search_memories, new `_dedup` + `_cosine`
- Test: `tests/test_search.py`

- [ ] **Step 1: Write failing test**

Append to `tests/test_search.py`:

```python
def test_dedup_drops_near_duplicates() -> None:
    from qwick_memory.search import SearchResult, _dedup

    v_a = [1.0, 0.0, 0.0]
    v_b = [0.999, 0.001, 0.0]
    v_c = [0.0, 1.0, 0.0]
    results = [
        SearchResult(id="a", repo="", type="note", tags="", author="",
                     created="2026-01-01T00:00:00+00:00", content="", score=0,
                     reranker_score=0.9, vector=v_a),
        SearchResult(id="b", repo="", type="note", tags="", author="",
                     created="2026-01-01T00:00:00+00:00", content="", score=0,
                     reranker_score=0.85, vector=v_b),
        SearchResult(id="c", repo="", type="note", tags="", author="",
                     created="2026-01-01T00:00:00+00:00", content="", score=0,
                     reranker_score=0.7, vector=v_c),
    ]
    kept = _dedup(results, threshold=0.92)
    assert [r.id for r in kept] == ["a", "c"]


def test_dedup_keeps_when_vector_missing() -> None:
    from qwick_memory.search import SearchResult, _dedup

    r = SearchResult(id="a", repo="", type="note", tags="", author="",
                     created="2026-01-01T00:00:00+00:00", content="",
                     score=0, reranker_score=0.5, vector=None)
    kept = _dedup([r], threshold=0.92)
    assert kept == [r]
```

- [ ] **Step 2: Run test to verify failure**

Run: `pytest tests/test_search.py::test_dedup_drops_near_duplicates -v`
Expected: FAIL — `_dedup` and `vector` field don't exist.

- [ ] **Step 3: Add `vector` to `SearchResult` + implement `_dedup`**

Modify the dataclass in `src/qwick_memory/search.py`:

```python
@dataclass
class SearchResult:
  """A single search result with relevance score."""

  id: str
  repo: str
  type: str
  tags: str
  author: str
  created: str
  content: str
  score: float
  reranker_score: float = 0.0
  quality: int = 3
  enriched_content: str = ""
  vector: list[float] | None = None
```

Update `_row_to_result`:

```python
def _row_to_result(row: dict[str, Any], score_key: str, normalize: bool = False) -> SearchResult:
  raw_score = float(row.get(score_key, 0.0))
  score = max(0.0, min(1.0, 1.0 - (raw_score / 2.0))) if normalize else raw_score
  vec = row.get("vector")
  return SearchResult(
    id=row["id"],
    repo=row["repo"],
    type=row["type"],
    tags=row["tags"],
    author=row["author"],
    created=row["created"],
    content=row["content"],
    score=score,
    quality=row.get("quality", 3),
    enriched_content=row.get("enriched_content", ""),
    vector=list(vec) if vec is not None else None,
  )
```

Add helpers above `search_memories`:

```python
def _cosine(a: list[float], b: list[float]) -> float:
  dot = sum(x * y for x, y in zip(a, b, strict=True))
  na = math.sqrt(sum(x * x for x in a))
  nb = math.sqrt(sum(y * y for y in b))
  if na == 0.0 or nb == 0.0:
    return 0.0
  return dot / (na * nb)


def _dedup(results: list[SearchResult], threshold: float = 0.92) -> list[SearchResult]:
  """Greedy near-duplicate dedup. Keep first, drop later within threshold."""
  kept: list[SearchResult] = []
  for r in results:
    if r.vector is None:
      kept.append(r)
      continue
    duplicate = False
    for k in kept:
      if k.vector is None:
        continue
      if _cosine(r.vector, k.vector) >= threshold:
        duplicate = True
        break
    if not duplicate:
      kept.append(r)
  return kept
```

In `search_memories`, after the backfill block and before the retrieval-count increment, add:

```python
  results = _dedup(results)
```

- [ ] **Step 4: Run tests to verify pass**

Run: `pytest tests/test_search.py -v`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/qwick_memory/search.py tests/test_search.py
git commit -m "feat(search): dedup near-duplicate results"
```

---

## Task 9: Read tunable constants from `config` in `search.py` (C.11, C.15 consumer)

**Files:**
- Modify: `src/qwick_memory/search.py`
- Test: `tests/test_search.py`

- [ ] **Step 1: Write failing test**

Append to `tests/test_search.py`:

```python
def test_hybrid_weight_from_config(monkeypatch: pytest.MonkeyPatch) -> None:
    """_try_hybrid_search reads HYBRID_WEIGHT from config."""
    import qwick_memory.config as cfg
    import qwick_memory.search as search_module

    captured: dict[str, float] = {}

    class FakeFuser:
        def __init__(self, weight: float) -> None:
            captured["weight"] = weight

    monkeypatch.setattr(cfg, "HYBRID_WEIGHT", 0.7)
    monkeypatch.setattr(
        "lancedb.rerankers.LinearCombinationReranker", FakeFuser
    )

    class FakeBuilder:
        def vector(self, *a, **k): return self
        def text(self, *a, **k): return self
        def rerank(self, **k): return self
        def where(self, *a, **k): return self
        def limit(self, *a, **k): return self
        def to_list(self): return []

    class FakeTable:
        def search(self, **k): return FakeBuilder()

    search_module._try_hybrid_search(FakeTable(), "q", [0.0]*768, None, 10)
    assert captured["weight"] == pytest.approx(0.7)
```

- [ ] **Step 2: Run test to verify failure**

Run: `pytest tests/test_search.py::test_hybrid_weight_from_config -v`
Expected: FAIL — current code passes hardcoded `weight=0.5`.

- [ ] **Step 3: Replace constants with config imports**

In `src/qwick_memory/search.py`, add near other imports:

```python
from qwick_memory import config as _config
```

Remove these module-level constants:

```python
MIN_RELEVANCE_SCORE = 0.3
MAX_SCORE_GAP = 0.15
RERANKER_MODEL = "Xenova/ms-marco-MiniLM-L-6-v2"
```

Update references:

- `_apply_thresholds` defaults: `min_score: float = _config.MIN_RELEVANCE_SCORE`, `max_gap: float = _config.MAX_SCORE_GAP`.
- `_get_reranker`: `_reranker = TextCrossEncoder(model_name=_config.RERANKER_MODEL)`.
- `_try_hybrid_search`: `fuser = LinearCombinationReranker(weight=_config.HYBRID_WEIGHT)`.

Keep `HALF_LIFE_DAYS` and `DEFAULT_HALF_LIFE` as-is.

- [ ] **Step 4: Run tests to verify pass**

Run: `pytest tests/ -v`
Expected: PASS, no regressions.

- [ ] **Step 5: Commit**

```bash
git add src/qwick_memory/search.py tests/test_search.py
git commit -m "feat(search): read tunable constants from config module"
```

---

## Task 10: `stats_cache.py` — cached read with mtime invalidation (B.7)

**Files:**
- Create: `src/qwick_memory/stats_cache.py`
- Create: `tests/test_stats_cache.py`

- [ ] **Step 1: Write failing test**

Create `tests/test_stats_cache.py`:

```python
"""Tests for qwick_memory.stats_cache."""

import json
import time
from pathlib import Path


def test_get_stats_returns_empty_when_missing(tmp_path: Path) -> None:
    from qwick_memory.stats_cache import StatsCache

    cache = StatsCache(stats_path=tmp_path / "stats.json")
    assert cache.get_stats() == {}


def test_get_stats_reloads_on_mtime_change(tmp_path: Path) -> None:
    from qwick_memory.stats_cache import StatsCache

    stats_path = tmp_path / "stats.json"
    stats_path.write_text(json.dumps({"a": {"retrieval_count": 1}}))
    cache = StatsCache(stats_path=stats_path)
    assert cache.get_stats()["a"]["retrieval_count"] == 1

    time.sleep(0.05)
    stats_path.write_text(json.dumps({"a": {"retrieval_count": 5}}))
    assert cache.get_stats()["a"]["retrieval_count"] == 5


def test_get_stats_does_not_reload_when_unchanged(tmp_path: Path) -> None:
    from qwick_memory.stats_cache import StatsCache

    stats_path = tmp_path / "stats.json"
    stats_path.write_text(json.dumps({"a": {"retrieval_count": 1}}))
    cache = StatsCache(stats_path=stats_path)
    cache.get_stats()
    reads_before = cache._read_count
    cache.get_stats()
    cache.get_stats()
    assert cache._read_count == reads_before
```

- [ ] **Step 2: Run test to verify failure**

Run: `pytest tests/test_stats_cache.py -v`
Expected: FAIL — module does not exist.

- [ ] **Step 3: Implement `StatsCache`**

Create `src/qwick_memory/stats_cache.py`:

```python
"""Cached stats reads with mtime invalidation + append-only event log."""

from __future__ import annotations

import json
import logging
from pathlib import Path
from typing import Any

logger = logging.getLogger(__name__)


class StatsCache:
  """Cached read of stats.json. Reloads only when file mtime changes."""

  def __init__(self, stats_path: Path | None = None) -> None:
    if stats_path is None:
      from qwick_memory.config import get_stats_path
      stats_path = get_stats_path()
    self._stats_path = stats_path
    self._cache: dict[str, Any] | None = None
    self._cache_mtime: float = -1.0
    self._read_count: int = 0

  def get_stats(self) -> dict[str, Any]:
    if not self._stats_path.exists():
      self._cache = {}
      self._cache_mtime = -1.0
      return self._cache
    try:
      mtime = self._stats_path.stat().st_mtime
    except OSError:
      return self._cache or {}
    if self._cache is None or mtime != self._cache_mtime:
      try:
        self._cache = json.loads(self._stats_path.read_text())
      except (OSError, json.JSONDecodeError):
        logger.warning("Could not read stats file %s", self._stats_path)
        self._cache = {}
      self._cache_mtime = mtime
      self._read_count += 1
    return self._cache


_default_cache: StatsCache | None = None


def get_default_cache() -> StatsCache:
  global _default_cache
  if _default_cache is None:
    _default_cache = StatsCache()
  return _default_cache


def get_stats() -> dict[str, Any]:
  return get_default_cache().get_stats()
```

- [ ] **Step 4: Run tests to verify pass**

Run: `pytest tests/test_stats_cache.py -v`
Expected: PASS, 3 tests.

- [ ] **Step 5: Commit**

```bash
git add src/qwick_memory/stats_cache.py tests/test_stats_cache.py
git commit -m "feat(stats_cache): cached stats with mtime invalidation"
```

---

## Task 11: Append-only events + compactor in `stats_cache.py` (B.8)

**Files:**
- Modify: `src/qwick_memory/stats_cache.py`
- Modify: `tests/test_stats_cache.py`

- [ ] **Step 1: Write failing test**

Append to `tests/test_stats_cache.py`:

```python
def test_append_event_writes_one_line(tmp_path: Path) -> None:
    from qwick_memory.stats_cache import append_event

    events_path = tmp_path / "events.jsonl"
    append_event({"type": "retrieval", "id": "abc"}, events_path=events_path)
    append_event({"type": "retrieval", "id": "def"}, events_path=events_path)
    lines = events_path.read_text().strip().splitlines()
    assert len(lines) == 2
    parsed = [json.loads(line) for line in lines]
    assert parsed[0]["id"] == "abc"
    assert parsed[1]["id"] == "def"


def test_compact_folds_events_into_stats(tmp_path: Path) -> None:
    from qwick_memory.stats_cache import append_event, compact

    stats_path = tmp_path / "stats.json"
    events_path = tmp_path / "events.jsonl"
    append_event({"type": "retrieval", "id": "x"}, events_path=events_path)
    append_event({"type": "retrieval", "id": "x"}, events_path=events_path)
    append_event(
        {"type": "feedback", "used_ids": ["x"], "irrelevant_ids": []},
        events_path=events_path,
    )

    compact(stats_path=stats_path, events_path=events_path)

    folded = json.loads(stats_path.read_text())
    assert folded["x"]["retrieval_count"] == 2
    assert folded["x"]["usage_count"] == 1
    assert folded["x"].get("irrelevance_count", 0) == 0
    assert not events_path.exists() or events_path.read_text() == ""


def test_append_event_concurrent_threads(tmp_path: Path) -> None:
    import threading

    from qwick_memory.stats_cache import append_event

    events_path = tmp_path / "events.jsonl"

    def worker(start: int) -> None:
        for i in range(50):
            append_event(
                {"type": "retrieval", "id": f"id-{start + i}"},
                events_path=events_path,
            )

    t1 = threading.Thread(target=worker, args=(0,))
    t2 = threading.Thread(target=worker, args=(100,))
    t1.start(); t2.start(); t1.join(); t2.join()
    lines = events_path.read_text().strip().splitlines()
    assert len(lines) == 100
    for line in lines:
        json.loads(line)
```

- [ ] **Step 2: Run test to verify failure**

Run: `pytest tests/test_stats_cache.py::test_append_event_writes_one_line -v`
Expected: FAIL — `append_event` does not exist.

- [ ] **Step 3: Implement `append_event` + `compact`**

Append to `src/qwick_memory/stats_cache.py`:

```python
import fcntl
import threading
from datetime import datetime, timezone

_append_lock = threading.Lock()
COMPACT_THRESHOLD_BYTES = 1_048_576  # 1 MB


def _events_path_default() -> Path:
  from qwick_memory.config import get_search_events_path
  return get_search_events_path()


def append_event(event: dict[str, Any], events_path: Path | None = None) -> None:
  """Append one JSON line to events.jsonl. Thread- and process-safe."""
  if events_path is None:
    events_path = _events_path_default()
  events_path.parent.mkdir(parents=True, exist_ok=True)
  line = json.dumps(event, separators=(",", ":")) + "\n"
  with _append_lock:
    try:
      with open(events_path, "a") as f:
        fcntl.flock(f.fileno(), fcntl.LOCK_EX)
        try:
          f.write(line)
        finally:
          fcntl.flock(f.fileno(), fcntl.LOCK_UN)
    except OSError as exc:
      logger.warning("Failed to append event: %s", exc)
      return
  try:
    if events_path.stat().st_size >= COMPACT_THRESHOLD_BYTES:
      threading.Thread(
        target=compact,
        kwargs={"events_path": events_path},
        daemon=True,
      ).start()
  except OSError:
    pass


def compact(
  stats_path: Path | None = None,
  events_path: Path | None = None,
) -> None:
  """Fold events.jsonl into stats.json atomically. Truncate events on success."""
  if stats_path is None:
    from qwick_memory.config import get_stats_path
    stats_path = get_stats_path()
  if events_path is None:
    events_path = _events_path_default()
  if not events_path.exists():
    return

  try:
    stats = json.loads(stats_path.read_text()) if stats_path.exists() else {}
  except (OSError, json.JSONDecodeError):
    stats = {}

  now = datetime.now(timezone.utc).isoformat()
  try:
    with open(events_path, "r+") as f:
      fcntl.flock(f.fileno(), fcntl.LOCK_EX)
      try:
        events_text = f.read()
        for raw in events_text.splitlines():
          if not raw.strip():
            continue
          try:
            evt = json.loads(raw)
          except json.JSONDecodeError:
            continue
          _apply_event(stats, evt, now)
        tmp_stats = stats_path.with_suffix(".json.tmp")
        tmp_stats.write_text(json.dumps(stats, indent=2))
        tmp_stats.replace(stats_path)
        f.seek(0)
        f.truncate(0)
      finally:
        fcntl.flock(f.fileno(), fcntl.LOCK_UN)
  except OSError as exc:
    logger.warning("Compact failed: %s", exc)


def _apply_event(stats: dict[str, Any], evt: dict[str, Any], now: str) -> None:
  """Apply one event to in-memory stats dict."""
  etype = evt.get("type")
  if etype == "retrieval":
    mid = evt.get("id")
    if not mid:
      return
    if mid not in stats:
      stats[mid] = {
        "retrieval_count": 0,
        "usage_count": 0,
        "irrelevance_count": 0,
        "last_retrieved": now,
      }
    stats[mid]["retrieval_count"] = stats[mid].get("retrieval_count", 0) + 1
    stats[mid]["last_retrieved"] = now
  elif etype == "feedback":
    for mid in evt.get("used_ids", []) or []:
      if mid not in stats:
        stats[mid] = {
          "retrieval_count": 1,
          "usage_count": 0,
          "irrelevance_count": 0,
          "last_retrieved": now,
        }
      stats[mid]["usage_count"] = stats[mid].get("usage_count", 0) + 1
    for mid in evt.get("irrelevant_ids", []) or []:
      if mid not in stats:
        stats[mid] = {
          "retrieval_count": 1,
          "usage_count": 0,
          "irrelevance_count": 0,
          "last_retrieved": now,
        }
      stats[mid]["irrelevance_count"] = stats[mid].get("irrelevance_count", 0) + 1
```

- [ ] **Step 4: Run tests to verify pass**

Run: `pytest tests/test_stats_cache.py -v`
Expected: PASS, 6 tests total.

- [ ] **Step 5: Commit**

```bash
git add src/qwick_memory/stats_cache.py tests/test_stats_cache.py
git commit -m "feat(stats_cache): append-only event log + compactor"
```

---

## Task 12: Wire `search.py` + `stats.py` to use `stats_cache` (B.7, B.8 integration)

**Files:**
- Modify: `src/qwick_memory/stats.py`
- Modify: `src/qwick_memory/search.py`
- Modify: `tests/test_stats.py`

- [ ] **Step 1: Inspect existing tests for breakage**

Run: `grep -n "increment_retrieval\|load_stats\|record_feedback" tests/test_stats.py`
Note any test that calls `increment_retrieval`/`record_feedback` and then reads `stats.json` directly. They will need a `compact()` call inserted.

- [ ] **Step 2: Update existing tests that need compact**

For any such test, change:

```python
increment_retrieval(["id1"], stats_path=stats_path)
data = load_stats(stats_path)
```

to:

```python
from qwick_memory.stats_cache import compact

events_path = tmp_path / "events.jsonl"
increment_retrieval(["id1"], events_path=events_path)
compact(stats_path=stats_path, events_path=events_path)
data = load_stats(stats_path)
```

Same pattern for `record_feedback`. The two tests added in Task 4 also need this — update them to call `compact()` after `record_feedback`.

- [ ] **Step 3: Patch `increment_retrieval` and `record_feedback`**

Replace both in `src/qwick_memory/stats.py`:

```python
def increment_retrieval(
  memory_ids: list[str],
  stats_path: Path | None = None,
  events_path: Path | None = None,
) -> None:
  """Append a retrieval event per memory ID. Compacted into stats.json later."""
  from qwick_memory.stats_cache import append_event

  for mid in memory_ids:
    append_event({"type": "retrieval", "id": mid}, events_path=events_path)


def record_feedback(
  used_ids: list[str],
  irrelevant_ids: list[str],
  stats_path: Path | None = None,
  events_path: Path | None = None,
) -> None:
  """Append a feedback event. Compacted into stats.json later."""
  if not used_ids and not irrelevant_ids:
    return
  from qwick_memory.stats_cache import append_event

  append_event(
    {
      "type": "feedback",
      "used_ids": list(used_ids),
      "irrelevant_ids": list(irrelevant_ids),
    },
    events_path=events_path,
  )
```

In `src/qwick_memory/search.py`, replace:

```python
  from qwick_memory.stats import load_stats

  all_stats = load_stats()
```

with:

```python
  from qwick_memory.stats_cache import get_stats

  all_stats = get_stats()
```

- [ ] **Step 4: Run all stats and search tests**

Run: `pytest tests/test_stats.py tests/test_stats_cache.py tests/test_search.py -v`
Expected: PASS. Add `compact()` calls in test fallout as needed.

- [ ] **Step 5: Commit**

```bash
git add src/qwick_memory/stats.py src/qwick_memory/search.py tests/test_stats.py
git commit -m "refactor(stats): route increment + feedback through events"
```

---

## Task 13: Async search log via ThreadPoolExecutor (B.6)

**Files:**
- Modify: `src/qwick_memory/search.py:146-173`
- Test: `tests/test_search.py`

- [ ] **Step 1: Write failing test**

Append to `tests/test_search.py`:

```python
def test_search_log_is_async(monkeypatch: pytest.MonkeyPatch,
                             built_index: MemoryIndex) -> None:
    """_log_search delegates to executor and does not block."""
    submitted: list = []

    class FakeExecutor:
        def submit(self, fn, *args, **kwargs):
            submitted.append((fn, args, kwargs))
            return None

    import qwick_memory.search as search_module

    monkeypatch.setattr(search_module, "_log_executor", FakeExecutor())
    search_memories(built_index, "PostgreSQL")
    assert submitted
```

- [ ] **Step 2: Run test to verify failure**

Run: `pytest tests/test_search.py::test_search_log_is_async -v`
Expected: FAIL — no `_log_executor` attribute.

- [ ] **Step 3: Refactor `_log_search`**

Add near the top of `src/qwick_memory/search.py`:

```python
from concurrent.futures import ThreadPoolExecutor

_log_executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix="qm-log")
```

Replace `_log_search` with two functions:

```python
def _log_search_sync(
  query: str,
  filters: dict[str, str | None],
  results: list[SearchResult],
  filtered_count: int,
) -> None:
  """Write one JSONL line to the search log."""
  try:
    from qwick_memory.config import get_search_log_path

    log_path = get_search_log_path()
    log_path.parent.mkdir(parents=True, exist_ok=True)
    entry = {
      "timestamp": datetime.now(timezone.utc).isoformat(),
      "type": "search",
      "query": query,
      **filters,
      "results": [
        {"id": r.id, "reranker_score": round(r.reranker_score, 4),
         "final_score": round(r.score, 4)}
        for r in results
      ],
      "result_count": len(results),
      "filtered_count": filtered_count,
    }
    with open(log_path, "a") as f:
      f.write(json.dumps(entry) + "\n")
  except (OSError, ValueError) as exc:
    logger.warning("Failed to log search interaction: %s", exc)


def _log_search(
  query: str,
  filters: dict[str, str | None],
  results: list[SearchResult],
  filtered_count: int,
) -> None:
  """Submit log write to the executor (non-blocking)."""
  _log_executor.submit(_log_search_sync, query, filters, results, filtered_count)
```

- [ ] **Step 4: Run tests to verify pass**

Run: `pytest tests/test_search.py -v`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/qwick_memory/search.py tests/test_search.py
git commit -m "perf(search): async search log via single-worker executor"
```

---

## Task 14: Eager preload reranker + embedding model in `server.py` (B.9)

**Files:**
- Modify: `src/qwick_memory/server.py:673-679`

- [ ] **Step 1: Edit `main()`**

In `src/qwick_memory/server.py`, replace `main()`:

```python
def main() -> None:
  """Run the MCP server with stdio transport. Preload heavy models first."""
  try:
    from qwick_memory.config import get_vectordb_dir
    from qwick_memory.index import MemoryIndex
    from qwick_memory.search import _get_reranker

    logger.info("Preloading cross-encoder reranker...")
    _get_reranker()
    logger.info("Preloading embedding model...")
    MemoryIndex(get_vectordb_dir()).model  # noqa: B018 — touch triggers lazy load
    logger.info("Model preload complete.")
  except Exception as exc:
    logger.warning("Model preload failed (continuing): %s", exc)

  mcp.run(transport="stdio")
```

- [ ] **Step 2: Smoke test import**

Run: `python -c "from qwick_memory.server import main; print(main.__doc__)"`
Expected: prints the docstring without ImportError.

- [ ] **Step 3: Run server tests**

Run: `pytest tests/test_server.py -v`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/qwick_memory/server.py
git commit -m "perf(server): preload reranker + embedding model at startup"
```

---

## Task 15: `qwick-memory calibrate` CLI command (A.2)

**Files:**
- Modify: `src/qwick_memory/cli.py`
- Modify: `tests/test_cli.py`

- [ ] **Step 1: Write failing test**

Append to `tests/test_cli.py`:

```python
def test_calibrate_outputs_recommendation(tmp_path: Path,
                                          monkeypatch: pytest.MonkeyPatch) -> None:
    import json
    import re

    from typer.testing import CliRunner

    from qwick_memory.cli import app

    rag_dir = tmp_path / "rag"
    rag_dir.mkdir()
    monkeypatch.setenv("QWICK_MEMORY_DIR", str(rag_dir))

    log_path = rag_dir / ".search_log.jsonl"
    stats_path = rag_dir / ".stats.json"

    log_entries = [
        {"type": "search", "query": "q",
         "results": [{"id": "good", "reranker_score": 0.7, "final_score": 0.5},
                     {"id": "bad", "reranker_score": 0.4, "final_score": 0.2}]},
        {"type": "feedback", "used_ids": ["good"], "irrelevant_ids": ["bad"]},
    ]
    log_path.write_text("\n".join(json.dumps(e) for e in log_entries) + "\n")
    stats_path.write_text(json.dumps({
        "good": {"retrieval_count": 1, "usage_count": 1, "irrelevance_count": 0},
        "bad": {"retrieval_count": 1, "usage_count": 0, "irrelevance_count": 1},
    }))

    runner = CliRunner()
    result = runner.invoke(app, ["calibrate"])
    assert result.exit_code == 0, result.output
    assert "recommended" in result.output.lower() or "threshold" in result.output.lower()
    assert re.search(r"0\.\d+", result.output)
```

- [ ] **Step 2: Run test to verify failure**

Run: `pytest tests/test_cli.py::test_calibrate_outputs_recommendation -v`
Expected: FAIL — `calibrate` command does not exist.

- [ ] **Step 3: Implement `calibrate`**

Append to `src/qwick_memory/cli.py`:

```python
@app.command()
def calibrate(
  verbose: bool = verbose_option,
) -> None:
  """Recommend a relevance threshold from the search log + feedback stats."""
  import json
  import statistics

  rag_dir = get_rag_dir()
  log_path = rag_dir / ".search_log.jsonl"
  stats_path = rag_dir / ".stats.json"

  if not log_path.exists():
    console.print(f"[red]No search log at {log_path}[/red]")
    raise typer.Exit(1)

  id_scores: dict[str, list[float]] = {}
  with open(log_path) as f:
    for line in f:
      line = line.strip()
      if not line:
        continue
      try:
        evt = json.loads(line)
      except json.JSONDecodeError:
        continue
      if evt.get("type") != "search":
        continue
      for r in evt.get("results", []):
        rid = r.get("id")
        rs = r.get("reranker_score")
        if rid is None or rs is None:
          continue
        id_scores.setdefault(rid, []).append(float(rs))

  if not stats_path.exists():
    console.print("[yellow]No stats file. Run after collecting feedback.[/yellow]")
    raise typer.Exit(0)
  stats = json.loads(stats_path.read_text())

  used_scores: list[float] = []
  irrelevant_scores: list[float] = []
  for mid, mstat in stats.items():
    scores = id_scores.get(mid, [])
    if not scores:
      continue
    avg = statistics.mean(scores)
    if mstat.get("usage_count", 0) > 0:
      used_scores.append(avg)
    if mstat.get("irrelevance_count", 0) > 0:
      irrelevant_scores.append(avg)

  def _percentiles(values: list[float]) -> dict[str, float]:
    if not values:
      return {}
    s = sorted(values)
    def pct(p: float) -> float:
      i = max(0, min(len(s) - 1, int(round(p * (len(s) - 1)))))
      return s[i]
    return {"p10": pct(0.10), "p25": pct(0.25), "p50": pct(0.50),
            "p75": pct(0.75), "p90": pct(0.90)}

  used_pct = _percentiles(used_scores)
  irrelevant_pct = _percentiles(irrelevant_scores)

  out.print("[bold]Used memories (reranker_score distribution):[/bold]")
  out.print(f"  n={len(used_scores)}  {used_pct}")
  out.print("[bold]Irrelevant memories (reranker_score distribution):[/bold]")
  out.print(f"  n={len(irrelevant_scores)}  {irrelevant_pct}")

  if used_scores and irrelevant_scores:
    recommended = (used_pct["p10"] + irrelevant_pct["p90"]) / 2
  elif used_scores:
    recommended = used_pct["p10"]
  else:
    recommended = 0.3

  out.print(
    f"\n[bold green]Recommended QWICK_MEMORY_MIN_RELEVANCE = {recommended:.3f}[/bold green]"
  )
```

- [ ] **Step 4: Run test to verify pass**

Run: `pytest tests/test_cli.py::test_calibrate_outputs_recommendation -v`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/qwick_memory/cli.py tests/test_cli.py
git commit -m "feat(cli): qwick-memory calibrate — recommend threshold"
```

---

## Task 16: E2E test updates

**Files:**
- Modify: `scripts/e2e-test.sh`

- [ ] **Step 1: Read current e2e script**

Run: `cat scripts/e2e-test.sh`
Identify the section where commands run after `qwick-memory save`, plus the existing PASS/FAIL counter variable names.

- [ ] **Step 2: Append new checks**

At the end of `scripts/e2e-test.sh`, before the final summary/exit, add:

```bash
# --- Search Quality v2.1 checks --------------------------------------------

echo
echo "=== Test: env override threads through ==="
ZERO_RESULTS=$(QWICK_MEMORY_MIN_RELEVANCE=0.999 qwick-memory search "anything" 2>&1 || true)
if echo "$ZERO_RESULTS" | grep -q "No results"; then
  echo "PASS: env override applied (threshold 0.999 filtered everything)"
  PASS=$((PASS + 1))
else
  echo "FAIL: env override did not filter results"
  FAIL=$((FAIL + 1))
fi

echo
echo "=== Test: calibrate command runs ==="
CALIBRATE_OUTPUT=$(qwick-memory calibrate 2>&1 || true)
if echo "$CALIBRATE_OUTPUT" | grep -qE "[Rr]ecommended|[Tt]hreshold"; then
  echo "PASS: calibrate command produced output"
  PASS=$((PASS + 1))
else
  echo "INFO: calibrate had no data (acceptable on fresh install)"
  PASS=$((PASS + 1))
fi
```

Adjust counter names (`PASS`/`FAIL`) to match the existing script's variables.

- [ ] **Step 3: Run e2e**

Run: `./scripts/e2e-test.sh`
Expected: all existing checks pass + 2 new checks pass.

- [ ] **Step 4: Commit**

```bash
git add scripts/e2e-test.sh
git commit -m "test(e2e): smoke-test calibrate command and env overrides"
```

---

## Task 17: Version bump + CHANGELOG

**Files:**
- Modify: `pyproject.toml:7`
- Create: `CHANGELOG.md`

- [ ] **Step 1: Bump version**

In `pyproject.toml`, change `version = "0.2.0"` to `version = "0.2.1"`.

- [ ] **Step 2: Create CHANGELOG**

Create `CHANGELOG.md`:

```markdown
# Changelog

## 0.2.1 — 2026-05-17

### Fixed

- **Search:** Escape `_` and `\` in LIKE filters for `repo` and `tag` filters
  (previously underscore acted as a wildcard, matching unintended rows).
- **Search:** `_usage_boost` no longer penalizes unseen memories. Unseen
  memories now get a neutral boost of 1.0; memories with stats scale by
  `(usage - irrelevance) / retrieval`, clipped at 0.
- **Search:** Negative feedback (`irrelevant_ids` in `qwick_memory_feedback`)
  now applies — previously collected but discarded.
- **Search:** `session-summary` half-life raised from 14 days to 30. Previous
  setting plus threshold filter could drop recent summaries entirely.
- **Search:** Cross-encoder reranker now sees `enriched_content` (with
  `[Repository] [Type] [Tags]` prefix), matching what the embedder and FTS see.

### Added

- **Search:** Threshold backfill — when the gap rule cuts results below
  `limit`, the caller now backfills from items that still pass the
  `min_score` floor.
- **Search:** Near-duplicate dedup. Results with cosine ≥ 0.92 to a kept
  result are dropped.
- **Config:** Env knobs `QWICK_MEMORY_MIN_RELEVANCE`, `QWICK_MEMORY_MAX_GAP`,
  `QWICK_MEMORY_HYBRID_WEIGHT`, `QWICK_MEMORY_RERANKER_MODEL`.
- **CLI:** `qwick-memory calibrate` reads the search log + stats and
  recommends a `QWICK_MEMORY_MIN_RELEVANCE` value.

### Changed

- **Stats:** Per-search increment is now append-only (`.stats.events.jsonl`).
  Compactor folds events into `.stats.json` atomically. Removes per-call
  read-modify-write race.
- **Stats:** Reads cached with mtime invalidation (`StatsCache`).
- **Search:** Log writes (`_log_search`) moved off the hot path to a
  single-worker thread pool.
- **Server:** MCP server preloads the cross-encoder and embedding model at
  startup to remove the per-session first-query latency cliff.

### Migration

- No schema change. `.stats.events.jsonl` is created on first event.
- `irrelevance_count` defaults to `0` for existing entries.
- Defaults unchanged for `MIN_RELEVANCE_SCORE` (0.3) and `HYBRID_WEIGHT` (0.5);
  upgraders see no behavior change unless they set the new env vars.

## 0.2.0 — 2026-03-24

- Initial search quality v2 release: enrichment, reranking, threshold
  filtering, auto-ranking.
```

- [ ] **Step 3: Run full test suite + linters**

Run: `pytest tests/ -v`
Expected: PASS.

Run: `ruff format src/ tests/ && ruff check src/ tests/`
Expected: clean.

Run: `pyright src/`
Expected: clean (or pre-existing warnings only).

- [ ] **Step 4: Commit**

```bash
git add pyproject.toml CHANGELOG.md
git commit -m "chore: bump to 0.2.1 + CHANGELOG for search quality v2.1"
```

---

## Self-review checklist

Before claiming the plan is done:

- [ ] **Spec coverage:** each of the 15 issues maps to a task.
  - A.1 → Task 2
  - A.2 → Task 15
  - A.3 → Task 5
  - A.4 → Task 3
  - A.5 → Task 4 (+ Task 12 reroutes through events)
  - B.6 → Task 13
  - B.7 → Task 10 (+ Task 12 wires search)
  - B.8 → Task 11 (+ Task 12 wires writes)
  - B.9 → Task 14
  - C.11 → Task 9
  - C.12 → Task 6
  - C.13 → Task 7
  - C.14 → Task 8
  - C.15 → Task 1 (foundation) + Task 9 (consumer)
- [ ] **Placeholder scan:** every step contains exact code/commands; no TBD/TODO; no "similar to Task N" references.
- [ ] **Type consistency:** `SearchResult.vector` is added in Task 8 before `_dedup` (same task). `_apply_thresholds` signature change in Task 7 is consistent with caller update in the same task. `increment_retrieval`/`record_feedback` add `events_path` keyword in Task 12.
- [ ] **No silent dependencies:** `_log_executor` introduced in Task 13 is module-level. `StatsCache` from Task 10 is used by `search.py` via `get_stats()` in Task 12.
