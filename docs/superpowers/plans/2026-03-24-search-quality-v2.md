# Search Quality v2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace garbage search results with precision retrieval — enriched embeddings, cross-encoder reranking, threshold filtering, quality scoring, usage feedback, freshness decay, and search logging.

**Architecture:** Nine changes in three tiers. Tier 1 (Tasks 1-5) fixes core retrieval: Memory model quality field, document enrichment, scoring functions, stats module, then pipeline wiring with cross-encoder reranking and tuned hybrid weights. Tier 2 (Tasks 6-9) adds auto-ranking: retrieval count increment, MCP feedback tool, search logging, migrate command. Tier 3 (Tasks 10-13) adds observability: CLI updates, test fixes, E2E tests, and final verification. Each tier produces a working system — later tiers enhance earlier ones.

**Tech Stack:** Python 3.10+, fastembed (TextEmbedding + TextCrossEncoder), LanceDB, Typer, FastMCP, pytest

**Spec:** `docs/superpowers/specs/2026-03-24-search-quality-v2-design.md`

---

## File Structure

| File | Responsibility | Action |
|---|---|---|
| `src/qwick_memory/index.py` | `_enrich_text()`, `_memory_to_record()` gains `quality` + `enriched_content`, FTS on `enriched_content`, `SCHEMA_VERSION` | Modify |
| `src/qwick_memory/search.py` | Reranker singleton, `_rerank()`, `_apply_thresholds()`, `_freshness_decay()`, `_compute_final_score()`, `_log_search()`, tuned hybrid | Modify |
| `src/qwick_memory/memory.py` | `quality` field on Memory, `parse_memory()` reads with fallback, `write_memory()` includes it | Modify |
| `src/qwick_memory/stats.py` | `load_stats()`, `save_stats()`, `increment_retrieval()`, `record_feedback()` | Create |
| `src/qwick_memory/config.py` | `get_stats_path()`, `get_search_log_path()` | Modify |
| `src/qwick_memory/server.py` | `qwick_memory_feedback` tool, `quality` on save, PROTOCOL update, tiered thresholds | Modify |
| `src/qwick_memory/cli.py` | `--quality` on save, doctor checks stats, search display | Modify |
| `tests/conftest.py` | Update `sample_memories` fixture with `quality` field | Modify |
| `tests/test_enrich.py` | Unit tests for `_enrich_text` | Create |
| `tests/test_scoring.py` | Unit tests for `_apply_thresholds`, `_freshness_decay`, `_compute_final_score` | Create |
| `tests/test_stats.py` | Unit tests for stats module | Create |
| `tests/test_search.py` | Update existing tests, add reranking/threshold integration tests | Modify |
| `tests/test_index.py` | Add enrichment + schema version tests | Modify |
| `tests/test_memory.py` | Add quality backward compat test | Modify |
| `tests/test_server.py` | Add feedback tool + quality param tests | Modify |
| `scripts/e2e-test.sh` | New checks for quality, thresholds, feedback, schema version | Modify |

---

## Tier 1: Core Retrieval Fixes

### Task 1: Add `quality` field to Memory model

**Files:**
- Modify: `src/qwick_memory/memory.py:46-59` (Memory dataclass)
- Modify: `src/qwick_memory/memory.py:62-80` (write_memory)
- Modify: `src/qwick_memory/memory.py:83-141` (parse_memory)
- Modify: `tests/conftest.py:19-55` (sample_memories fixture)
- Modify: `tests/test_memory.py`

- [ ] **Step 1: Write failing test for quality field on Memory**

In `tests/test_memory.py`, add:

```python
def test_memory_quality_default():
  """Memory quality defaults to 3."""
  mem = Memory(
    id="test_q",
    repo=["test"],
    type="note",
    tags=[],
    author="tester",
    created=datetime(2026, 1, 1),
    content="test",
  )
  assert mem.quality == 3


def test_memory_quality_explicit():
  """Memory quality can be set explicitly."""
  mem = Memory(
    id="test_q",
    repo=["test"],
    type="note",
    tags=[],
    author="tester",
    created=datetime(2026, 1, 1),
    content="test",
    quality=5,
  )
  assert mem.quality == 5
```

- [ ] **Step 2: Run test to verify it fails**

Run: `uv run pytest tests/test_memory.py::test_memory_quality_default -v`
Expected: FAIL — `TypeError: unexpected keyword argument 'quality'`

- [ ] **Step 3: Add quality field to Memory dataclass**

In `src/qwick_memory/memory.py`, add `quality` field to the `Memory` dataclass. It must be declared BEFORE `content_hash` (which is `field(init=False)`) but has a default so it doesn't break existing callers:

```python
@dataclass
class Memory:
  """A single unit of knowledge stored in qwick-memory."""

  id: str
  repo: list[str]
  type: MemoryType
  tags: list[str]
  author: str
  created: datetime
  content: str
  quality: int = 3
  content_hash: str = field(init=False)

  def __post_init__(self) -> None:
    self.content_hash = generate_id(self.content)
```

- [ ] **Step 4: Run test to verify it passes**

Run: `uv run pytest tests/test_memory.py::test_memory_quality_default tests/test_memory.py::test_memory_quality_explicit -v`
Expected: PASS

- [ ] **Step 5: Write failing test for parse_memory quality backward compat**

In `tests/test_memory.py`, add:

```python
def test_parse_memory_without_quality_defaults_to_3(tmp_path):
  """Existing memories without quality field default to 3."""
  md = tmp_path / "old.md"
  md.write_text(
    "---\n"
    "id: old001\n"
    "repo: [test]\n"
    "type: note\n"
    "tags: []\n"
    "author: alice\n"
    "created: 2026-01-01T00:00:00\n"
    "content_hash: old001\n"
    "---\n"
    "Old memory without quality field.\n"
  )
  mem = parse_memory(md)
  assert mem.quality == 3


def test_parse_memory_with_quality(tmp_path):
  """Memories with quality field parse correctly."""
  md = tmp_path / "new.md"
  md.write_text(
    "---\n"
    "id: new001\n"
    "repo: [test]\n"
    "type: note\n"
    "tags: []\n"
    "author: alice\n"
    "created: 2026-01-01T00:00:00\n"
    "quality: 5\n"
    "content_hash: new001\n"
    "---\n"
    "New memory with quality.\n"
  )
  mem = parse_memory(md)
  assert mem.quality == 5
```

- [ ] **Step 6: Run test to verify it fails**

Run: `uv run pytest tests/test_memory.py::test_parse_memory_without_quality_defaults_to_3 -v`
Expected: FAIL — `parse_memory` does not read `quality`

- [ ] **Step 7: Update parse_memory to read quality with fallback**

In `src/qwick_memory/memory.py`, inside the `try` block of `parse_memory()`, after the `repo_list` logic and before the `return Memory(...)`, add quality extraction:

```python
    quality = int(post.metadata.get("quality", 3))
```

And pass it to the Memory constructor:

```python
    return Memory(
      id=str(post.metadata["id"]),
      repo=repo_list,
      type=mem_type,
      tags=[str(t) for t in list(post.metadata["tags"])],
      author=str(post.metadata["author"]),
      created=created,
      content=post.content,
      quality=quality,
    )
```

- [ ] **Step 8: Update write_memory to include quality**

In `src/qwick_memory/memory.py`, in `write_memory()`, add `quality=memory.quality` to the `frontmatter.Post(...)` call.

- [ ] **Step 9: Run all memory tests**

Run: `uv run pytest tests/test_memory.py -v`
Expected: ALL PASS

- [ ] **Step 10: Update conftest.py sample_memories fixture**

Add `quality` to each sample memory in `tests/conftest.py`:

```python
Memory(
  id="mem_pg_001",
  repo=["acme/backend"],
  type="decision",
  tags=["database", "postgresql"],
  author="alice",
  created=datetime(2026, 1, 15, 10, 0, 0),
  content="We chose PostgreSQL as the primary database for its JSONB support and strong ecosystem.",
  quality=4,
),
Memory(
  id="mem_sess_002",
  repo=["acme/backend"],
  type="bug",
  tags=["auth", "session"],
  author="bob",
  created=datetime(2026, 2, 1, 14, 30, 0),
  content="Session tokens were not being invalidated on logout due to a missing Redis DEL call.",
  quality=5,
),
Memory(
  id="mem_react_003",
  repo=["acme/frontend"],
  type="convention",
  tags=["react", "exports"],
  author="carol",
  created=datetime(2026, 2, 20, 9, 0, 0),
  content="All React components must use named exports, not default exports, for better tree-shaking.",
  quality=3,
),
```

- [ ] **Step 11: Run full test suite to verify nothing breaks**

Run: `uv run pytest -v`
Expected: ALL PASS

- [ ] **Step 12: Commit**

```bash
git add src/qwick_memory/memory.py tests/conftest.py tests/test_memory.py
git commit -m "feat: add quality field to Memory model with backward compat"
```

---

### Task 2: Document enrichment + schema versioning in index.py

**Files:**
- Modify: `src/qwick_memory/index.py`
- Create: `tests/test_enrich.py`
- Modify: `tests/test_index.py`

- [ ] **Step 1: Write failing tests for `_enrich_text`**

Create `tests/test_enrich.py`:

```python
"""Tests for document enrichment — pure function, no MemoryIndex needed."""

from datetime import datetime

from qwick_memory.index import _enrich_text
from qwick_memory.memory import Memory


def test_enrich_text_basic():
  """Enriched text includes repo, type, tags, and content."""
  mem = Memory(
    id="e001", repo=["sidegig-api"], type="bug",
    tags=["database", "postgres"], author="alice",
    created=datetime(2026, 1, 1), content="Connection pool timeout.",
  )
  result = _enrich_text(mem)
  assert "[Repository: sidegig-api]" in result
  assert "[Type: bug]" in result
  assert "[Tags: database, postgres]" in result
  assert "Connection pool timeout." in result


def test_enrich_text_empty_tags():
  """Empty tags omits the [Tags: ] block entirely."""
  mem = Memory(
    id="e002", repo=["test"], type="note",
    tags=[], author="alice",
    created=datetime(2026, 1, 1), content="No tags here.",
  )
  result = _enrich_text(mem)
  assert "[Tags:" not in result
  assert "No tags here." in result


def test_enrich_text_multi_repo():
  """Multiple repos are comma-separated."""
  mem = Memory(
    id="e003", repo=["api", "web"], type="decision",
    tags=["arch"], author="alice",
    created=datetime(2026, 1, 1), content="Shared config.",
  )
  result = _enrich_text(mem)
  assert "[Repository: api, web]" in result
```

- [ ] **Step 2: Run test to verify it fails**

Run: `uv run pytest tests/test_enrich.py -v`
Expected: FAIL — `ImportError: cannot import name '_enrich_text'`

- [ ] **Step 3: Implement `_enrich_text` in index.py**

Add at module level in `src/qwick_memory/index.py` (after imports, before the class):

```python
SCHEMA_VERSION = 2


def _enrich_text(memory: Memory) -> str:
  """Prepend metadata to content for richer embeddings and FTS."""
  parts = [f"[Repository: {', '.join(memory.repo)}]"]
  parts.append(f"[Type: {memory.type}]")
  if memory.tags:
    parts.append(f"[Tags: {', '.join(memory.tags)}]")
  return f"{' '.join(parts)}\n{memory.content}"
```

- [ ] **Step 4: Run enrich tests to verify they pass**

Run: `uv run pytest tests/test_enrich.py -v`
Expected: ALL PASS

- [ ] **Step 5: Update `_memory_to_record` to include `enriched_content` and `quality`**

In `src/qwick_memory/index.py`, modify `_memory_to_record`:

```python
  def _memory_to_record(self, memory: Memory, vector: list[float]) -> dict[str, Any]:
    """Convert a Memory + its embedding vector into a flat dict for LanceDB."""
    return {
      "id": memory.id,
      "repo": ",".join(memory.repo),
      "type": memory.type,
      "tags": ",".join(memory.tags),
      "author": memory.author,
      "created": memory.created.isoformat(),
      "content": memory.content,
      "enriched_content": _enrich_text(memory),
      "quality": memory.quality,
      "content_hash": memory.content_hash,
      "vector": vector,
    }
```

- [ ] **Step 6: Update `_embed_documents` to use enriched text**

Change `_full_build` and `_incremental_build` to embed enriched content. In `_full_build`:

```python
    texts = [_enrich_text(mem) for mem in memories]
```

In `_incremental_build`, for updated memories:

```python
      vec = self._embed_documents([_enrich_text(mem)])[0]
```

And for new memories:

```python
      texts = [_enrich_text(m) for m in new_memories]
```

Also update `upsert`:

```python
  def upsert(self, memory: Memory) -> None:
    vectors = self._embed_documents([_enrich_text(memory)])
    record = self._memory_to_record(memory, vectors[0])
    ...
```

- [ ] **Step 7: Update FTS index to use `enriched_content`**

In `_create_table`:

```python
      table.create_fts_index("enriched_content", replace=True)
```

In `_incremental_build`:

```python
      table.create_fts_index("enriched_content", replace=True)
```

- [ ] **Step 8: Add schema versioning to meta.json**

Update `_read_meta` — no change needed (it reads whatever's in meta.json).

Update `_write_meta`:

```python
  def _write_meta(self) -> None:
    meta_path = self._vectordb_dir / "meta.json"
    meta_path.write_text(json.dumps({"model": MODEL_NAME, "schema_version": SCHEMA_VERSION}))
```

Add `schema_matches` method:

```python
  def schema_matches(self) -> bool:
    """Check if the indexed schema version matches the current SCHEMA_VERSION."""
    return self._current_meta.get("schema_version") == SCHEMA_VERSION
```

- [ ] **Step 9: Update migrate logic in `build()` to check schema version**

In `build()`, after the model check:

```python
    # Auto-force rebuild if schema version changed
    if not self.schema_matches() and not force:
      logger.info(
        "Schema version changed (%s → %s). Forcing full rebuild.",
        self._current_meta.get("schema_version", 1),
        SCHEMA_VERSION,
      )
      force = True
```

- [ ] **Step 10: Write test for schema version triggering rebuild**

In `tests/test_index.py`, add:

```python
def test_schema_version_mismatch_forces_rebuild(
  memories_dir: Path,
  vectordb_dir: Path,
) -> None:
  """When schema_version in meta.json doesn't match code, build forces full rebuild."""
  idx = MemoryIndex(vectordb_dir)
  idx.build(memories_dir)
  assert idx.count() == 3

  # Write old schema version
  meta_path = vectordb_dir / "meta.json"
  meta = json.loads(meta_path.read_text())
  meta["schema_version"] = 1
  meta_path.write_text(json.dumps(meta))

  # Rebuild — should detect mismatch and force full rebuild
  idx2 = MemoryIndex(vectordb_dir)
  stats = idx2.build(memories_dir)
  assert stats["new"] == 3  # full rebuild, not incremental
```

- [ ] **Step 11: Run all index + enrich tests**

Run: `uv run pytest tests/test_enrich.py tests/test_index.py -v`
Expected: ALL PASS

- [ ] **Step 12: Run full test suite**

Run: `uv run pytest -v`
Expected: ALL PASS (existing search tests may need updating — the enriched content changes FTS behavior)

- [ ] **Step 13: Commit**

```bash
git add src/qwick_memory/index.py tests/test_enrich.py tests/test_index.py
git commit -m "feat: document enrichment, enriched_content column, schema versioning"
```

---

### Task 3: Scoring functions — thresholds, freshness, combined score

**Files:**
- Modify: `src/qwick_memory/search.py`
- Create: `tests/test_scoring.py`

- [ ] **Step 1: Write failing tests for scoring functions**

Create `tests/test_scoring.py`:

```python
"""Tests for scoring functions — thresholds, freshness decay, combined score."""

import math
from datetime import datetime, timezone

from qwick_memory.search import (
  _apply_thresholds,
  _compute_final_score,
  _freshness_decay,
  SearchResult,
)


def _make_result(score: float, reranker_score: float = 0.0, **kwargs) -> SearchResult:
  defaults = dict(
    id="x", repo="r", type="note", tags="", author="a",
    created="2026-01-01T00:00:00", content="c", quality=3,
  )
  defaults.update(kwargs)
  return SearchResult(score=score, reranker_score=reranker_score, **defaults)


# -- _apply_thresholds --

def test_apply_thresholds_filters_below_floor():
  """Results below min_score are removed."""
  results = [_make_result(0.0, 0.8), _make_result(0.0, 0.2), _make_result(0.0, 0.1)]
  filtered = _apply_thresholds(results, min_score=0.3, max_gap=0.15)
  assert len(filtered) == 1
  assert filtered[0].reranker_score == 0.8


def test_apply_thresholds_gap_detection():
  """Large gap between consecutive scores triggers cutoff."""
  results = [_make_result(0.0, 0.82), _make_result(0.0, 0.79), _make_result(0.0, 0.41), _make_result(0.0, 0.38)]
  filtered = _apply_thresholds(results, min_score=0.3, max_gap=0.15)
  assert len(filtered) == 2


def test_apply_thresholds_empty_list():
  """Empty input returns empty output."""
  assert _apply_thresholds([], min_score=0.3, max_gap=0.15) == []


def test_apply_thresholds_single_above():
  """Single result above threshold passes."""
  results = [_make_result(0.0, 0.5)]
  assert len(_apply_thresholds(results, min_score=0.3, max_gap=0.15)) == 1


def test_apply_thresholds_all_below():
  """All results below threshold returns empty."""
  results = [_make_result(0.0, 0.1), _make_result(0.0, 0.05)]
  assert _apply_thresholds(results, min_score=0.3, max_gap=0.15) == []


# -- _freshness_decay --

def test_freshness_decay_convention_365_half_life():
  """Convention at 365 days should be ~0.5."""
  from datetime import timedelta
  created = datetime.now(timezone.utc) - timedelta(days=365)
  decay = _freshness_decay(created, "convention")
  assert 0.45 < decay < 0.55


def test_freshness_decay_session_summary_14_half_life():
  """Session summary at 14 days should be ~0.5."""
  from datetime import timedelta
  created = datetime.now(timezone.utc) - timedelta(days=14)
  decay = _freshness_decay(created, "session-summary")
  assert 0.45 < decay < 0.55


def test_freshness_decay_brand_new():
  """Memory created now should have decay ~1.0."""
  created = datetime.now(timezone.utc)
  decay = _freshness_decay(created, "note")
  assert decay > 0.99


def test_freshness_decay_unknown_type_uses_default():
  """Unknown memory type falls back to 90-day half-life."""
  from datetime import timedelta
  created = datetime.now(timezone.utc) - timedelta(days=90)
  decay = _freshness_decay(created, "unknown_type")
  assert 0.45 < decay < 0.55


# -- _compute_final_score --

def test_compute_final_score_all_perfect():
  """All signals at max → final score equals reranker score."""
  score = _compute_final_score(
    reranker_score=0.9,
    memory_type="convention",
    created=datetime.now(timezone.utc),
    quality=5,
    stats=None,
  )
  # freshness ~1.0, quality_boost = 1.0, usage_boost = 0.9 (default)
  assert 0.8 < score < 0.95


def test_compute_final_score_low_quality():
  """Low quality deprioritizes but doesn't zero out."""
  score = _compute_final_score(
    reranker_score=0.9,
    memory_type="convention",
    created=datetime.now(timezone.utc),
    quality=1,
    stats=None,
  )
  # quality_boost = 0.68
  assert 0.5 < score < 0.7
```

- [ ] **Step 2: Run test to verify it fails**

Run: `uv run pytest tests/test_scoring.py -v`
Expected: FAIL — `ImportError`

- [ ] **Step 3: Update SearchResult dataclass**

In `src/qwick_memory/search.py`, update:

```python
@dataclass
class SearchResult:
  """A single search result with relevance score.

  Score lifecycle: before reranking, score = raw similarity.
  After pipeline: score = composite final score (reranker * freshness * quality * usage).
  """

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
  enriched_content: str = ""  # For cross-encoder reranking (includes metadata prefix)
```

- [ ] **Step 3b: Add required imports to search.py**

Add these to the module-level imports in `src/qwick_memory/search.py`:

```python
import json
import math
from datetime import datetime, timezone
```

These are needed by `_apply_thresholds`, `_freshness_decay`, `_compute_final_score`, `_rerank` (sigmoid), and `_log_search`.

- [ ] **Step 4: Implement `_apply_thresholds`**

In `src/qwick_memory/search.py`, add constants and function:

```python
MIN_RELEVANCE_SCORE = 0.3
MAX_SCORE_GAP = 0.15


def _apply_thresholds(
  results: list[SearchResult],
  min_score: float = MIN_RELEVANCE_SCORE,
  max_gap: float = MAX_SCORE_GAP,
) -> list[SearchResult]:
  """Filter results by hard floor and gap detection on reranker_score."""
  if not results:
    return []

  # Hard floor
  above = [r for r in results if r.reranker_score >= min_score]
  if not above:
    return []

  # Results should already be sorted by reranker_score descending
  above.sort(key=lambda r: r.reranker_score, reverse=True)

  # Gap detection
  filtered = [above[0]]
  for i in range(1, len(above)):
    gap = above[i - 1].reranker_score - above[i].reranker_score
    if gap > max_gap:
      break
    filtered.append(above[i])

  return filtered
```

- [ ] **Step 5: Implement `_freshness_decay`**

```python
import math
from datetime import datetime, timezone

HALF_LIFE_DAYS: dict[str, int] = {
  "convention": 365,
  "preference": 365,
  "decision": 180,
  "pattern": 180,
  "discovery": 120,
  "bug": 90,
  "note": 60,
  "session-summary": 14,
}
DEFAULT_HALF_LIFE = 90


def _freshness_decay(created: datetime, memory_type: str) -> float:
  """Exponential decay based on memory age and type-specific half-life."""
  half_life = HALF_LIFE_DAYS.get(memory_type, DEFAULT_HALF_LIFE)
  now = datetime.now(timezone.utc)
  if created.tzinfo is None:
    created = created.replace(tzinfo=timezone.utc)
  age_days = max(0.0, (now - created).total_seconds() / 86400)
  return math.exp(-math.log(2) / half_life * age_days)
```

- [ ] **Step 6: Implement `_compute_final_score`**

```python
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
  # Usage boost
  if stats is not None:
    retrieval_count = stats.get("retrieval_count", 0)
    usage_count = stats.get("usage_count", 0)
    usage_boost = 0.8 + 0.2 * (usage_count / max(1, retrieval_count))
  else:
    usage_boost = 0.9  # neutral-positive for new memories
  return reranker_score * freshness * quality_boost * usage_boost
```

- [ ] **Step 7: Run scoring tests**

Run: `uv run pytest tests/test_scoring.py -v`
Expected: ALL PASS

- [ ] **Step 8: Commit**

```bash
git add src/qwick_memory/search.py tests/test_scoring.py
git commit -m "feat: add scoring functions — thresholds, freshness decay, combined score"
```

---

### Task 4: Stats module + config helpers

> **Note:** This task must come BEFORE the pipeline wiring task because `search_memories` imports `load_stats`.

**Files:**
- Create: `src/qwick_memory/stats.py`
- Modify: `src/qwick_memory/config.py`
- Create: `tests/test_stats.py`

- [ ] **Step 1: Write failing tests for stats module**

Create `tests/test_stats.py`:

```python
"""Tests for qwick_memory.stats — usage tracking with atomic writes."""

import json
from pathlib import Path

from qwick_memory.stats import (
  increment_retrieval,
  load_stats,
  record_feedback,
  save_stats,
)


def test_load_stats_missing_file(tmp_path):
  """Loading from non-existent file returns empty dict."""
  stats = load_stats(tmp_path / "nope.json")
  assert stats == {}


def test_load_stats_corrupted_file(tmp_path):
  """Loading from corrupted file returns empty dict."""
  path = tmp_path / "bad.json"
  path.write_text("not json {{{")
  stats = load_stats(path)
  assert stats == {}


def test_save_and_load_roundtrip(tmp_path):
  """Save then load returns same data."""
  path = tmp_path / "stats.json"
  data = {"abc123": {"retrieval_count": 5, "usage_count": 3, "last_retrieved": "2026-03-24T00:00:00"}}
  save_stats(data, path)
  loaded = load_stats(path)
  assert loaded == data


def test_increment_retrieval(tmp_path):
  """increment_retrieval creates entry and bumps count."""
  path = tmp_path / "stats.json"
  increment_retrieval(["id1", "id2"], path)
  stats = load_stats(path)
  assert stats["id1"]["retrieval_count"] == 1
  assert stats["id2"]["retrieval_count"] == 1

  increment_retrieval(["id1"], path)
  stats = load_stats(path)
  assert stats["id1"]["retrieval_count"] == 2


def test_record_feedback(tmp_path):
  """record_feedback increments usage_count for used_ids."""
  path = tmp_path / "stats.json"
  increment_retrieval(["id1", "id2"], path)
  record_feedback(used_ids=["id1"], irrelevant_ids=["id2"], stats_path=path)
  stats = load_stats(path)
  assert stats["id1"]["usage_count"] == 1
  assert stats["id2"]["usage_count"] == 0
```

- [ ] **Step 2: Run test to verify it fails**

Run: `uv run pytest tests/test_stats.py -v`
Expected: FAIL — `ModuleNotFoundError: No module named 'qwick_memory.stats'`

- [ ] **Step 3: Add config helpers**

In `src/qwick_memory/config.py`, add:

```python
def get_stats_path() -> Path:
  return get_rag_dir() / ".stats.json"


def get_search_log_path() -> Path:
  return get_rag_dir() / ".search_log.jsonl"
```

- [ ] **Step 4: Implement stats module**

Create `src/qwick_memory/stats.py`:

```python
"""Usage statistics for qwick-memory: retrieval counts and feedback tracking."""

from __future__ import annotations

import json
import logging
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

logger = logging.getLogger(__name__)


def load_stats(stats_path: Path | None = None) -> dict[str, Any]:
  """Load stats from JSON file. Returns empty dict on missing/corrupted file."""
  if stats_path is None:
    from qwick_memory.config import get_stats_path
    stats_path = get_stats_path()
  if not stats_path.exists():
    return {}
  try:
    return json.loads(stats_path.read_text())
  except (json.JSONDecodeError, OSError):
    logger.warning("Could not read stats file %s; returning empty stats.", stats_path)
    return {}


def save_stats(data: dict[str, Any], stats_path: Path | None = None) -> None:
  """Atomically write stats to JSON file (temp file then rename)."""
  if stats_path is None:
    from qwick_memory.config import get_stats_path
    stats_path = get_stats_path()
  stats_path.parent.mkdir(parents=True, exist_ok=True)
  tmp = stats_path.with_suffix(".tmp")
  try:
    tmp.write_text(json.dumps(data, indent=2))
    tmp.rename(stats_path)
  except OSError:
    logger.warning("Failed to write stats file %s", stats_path)
    tmp.unlink(missing_ok=True)


def increment_retrieval(memory_ids: list[str], stats_path: Path | None = None) -> None:
  """Increment retrieval_count for each memory ID."""
  stats = load_stats(stats_path)
  now = datetime.now(timezone.utc).isoformat()
  for mid in memory_ids:
    if mid not in stats:
      stats[mid] = {"retrieval_count": 0, "usage_count": 0, "last_retrieved": now}
    stats[mid]["retrieval_count"] += 1
    stats[mid]["last_retrieved"] = now
  save_stats(stats, stats_path)


def record_feedback(
  used_ids: list[str],
  irrelevant_ids: list[str],
  stats_path: Path | None = None,
) -> None:
  """Record which memories were used vs irrelevant."""
  stats = load_stats(stats_path)
  for mid in used_ids:
    if mid not in stats:
      stats[mid] = {"retrieval_count": 1, "usage_count": 0, "last_retrieved": ""}
    stats[mid]["usage_count"] += 1
  save_stats(stats, stats_path)
```

Note: `get_stats_path` is lazy-imported inside each function to avoid module-level coupling.

- [ ] **Step 5: Run stats tests**

Run: `uv run pytest tests/test_stats.py -v`
Expected: ALL PASS

- [ ] **Step 6: Commit**

```bash
git add src/qwick_memory/stats.py src/qwick_memory/config.py tests/test_stats.py
git commit -m "feat: stats module for usage tracking + config path helpers"
```

---

### Task 5: Cross-encoder reranking + tuned hybrid weights

**Files:**
- Modify: `src/qwick_memory/search.py`
- Modify: `tests/test_search.py`

- [ ] **Step 1: Write failing integration test for reranking**

In `tests/test_search.py`, add:

```python
def test_search_scores_use_reranker(built_index: MemoryIndex) -> None:
  """After reranking, results have reranker_score > 0 for relevant queries."""
  results = search_memories(built_index, "PostgreSQL database")
  assert len(results) > 0
  for r in results:
    assert r.reranker_score > 0


def test_search_irrelevant_returns_empty(built_index: MemoryIndex) -> None:
  """Completely irrelevant query returns no results after threshold filtering."""
  results = search_memories(built_index, "xyzzy foobar blargh nonsense")
  assert results == []
```

- [ ] **Step 2: Run test to verify it fails**

Run: `uv run pytest tests/test_search.py::test_search_scores_use_reranker -v`
Expected: FAIL — `SearchResult` has no `reranker_score` attribute yet in results

- [ ] **Step 3: Add reranker lazy singleton**

At module level in `src/qwick_memory/search.py`:

```python
from fastembed.rerank.cross_encoder import TextCrossEncoder

RERANKER_MODEL = "Xenova/ms-marco-MiniLM-L-6-v2"
_reranker: TextCrossEncoder | None = None


def _get_reranker() -> TextCrossEncoder:
  """Lazy-load the cross-encoder reranker."""
  global _reranker
  if _reranker is None:
    _reranker = TextCrossEncoder(model_name=RERANKER_MODEL)
  return _reranker
```

- [ ] **Step 4: Implement `_rerank` function**

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

  # Sigmoid normalize raw logits to 0-1
  for result, raw in zip(results, raw_scores):
    result.reranker_score = 1.0 / (1.0 + math.exp(-raw))

  # Sort by reranker_score descending
  results.sort(key=lambda r: r.reranker_score, reverse=True)
  return results[:limit]
```

- [ ] **Step 5: Update `_row_to_result` to pass through quality**

```python
def _row_to_result(row: dict[str, Any], score_key: str, normalize: bool = False) -> SearchResult:
  raw_score = float(row.get(score_key, 0.0))
  score = max(0.0, min(1.0, 1.0 - (raw_score / 2.0))) if normalize else raw_score
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
  )
```

- [ ] **Step 6: Update `_try_hybrid_search` with tuned weights**

```python
from lancedb.rerankers import LinearCombinationReranker

def _try_hybrid_search(
  table: Any,
  query: str,
  query_vector: list[float],
  where_expr: str | None,
  limit: int,
) -> list[SearchResult] | None:
  try:
    reranker = LinearCombinationReranker(weight=0.5)
    builder = table.search(query_type="hybrid").vector(query_vector).text(query)
    builder = builder.rerank(reranker=reranker)
    if where_expr:
      builder = builder.where(where_expr)
    builder = builder.limit(limit)
    rows = builder.to_list()
    return [_row_to_result(row, score_key="_relevance_score", normalize=False) for row in rows]
  except Exception:
    logger.debug("Hybrid search failed; falling back to vector-only search.")
    return None
```

- [ ] **Step 7: Wire the full pipeline into `search_memories`**

Update `search_memories` to use the new pipeline:

```python
def search_memories(
  index: MemoryIndex,
  query: str,
  repo: str | None = None,
  type_filter: str | None = None,
  tag: str | None = None,
  limit: int = 10,
) -> list[SearchResult]:
  table = index._get_table()
  if table is None:
    return []

  query_vector = index._embed_query(query)

  # Build metadata filter
  where_clauses: list[str] = []
  if repo is not None:
    safe_repo = repo.replace('"', '\\"').replace("%", "")
    where_clauses.append(f'repo LIKE "%{safe_repo}%"')
  if type_filter is not None:
    safe_type = type_filter.replace('"', '\\"')
    where_clauses.append(f'type = "{safe_type}"')
  if tag is not None:
    safe_tag = tag.replace('"', '\\"').replace("%", "")
    where_clauses.append(f'tags LIKE "%{safe_tag}%"')
  where_expr = " AND ".join(where_clauses) if where_clauses else None

  # Step 1: Hybrid search (over-retrieve)
  retrieve_limit = max(limit * 2, 20)
  results = _try_hybrid_search(table, query, query_vector, where_expr, retrieve_limit)
  if results is None:
    results = _vector_search(table, query_vector, where_expr, retrieve_limit)

  if not results:
    return []

  # Step 2: Cross-encoder rerank
  results = _rerank(query, results, retrieve_limit)

  # Step 3: Threshold filter on reranker_score
  results = _apply_thresholds(results)

  if not results:
    return []

  # Step 4: Combined scoring
  from qwick_memory.stats import load_stats
  all_stats = load_stats()
  for r in results:
    created_dt = datetime.fromisoformat(r.created)
    mem_stats = all_stats.get(r.id)
    r.score = _compute_final_score(
      reranker_score=r.reranker_score,
      memory_type=r.type,
      created=created_dt,
      quality=r.quality,
      stats=mem_stats,
    )

  # Step 5: Sort by final_score, return top limit
  results.sort(key=lambda r: r.score, reverse=True)
  return results[:limit]
```

- [ ] **Step 8: Run search tests**

Run: `uv run pytest tests/test_search.py -v`
Expected: ALL PASS (some existing tests may need score assertion adjustments since the scoring pipeline changed)

- [ ] **Step 9: Fix any existing test failures**

The existing `test_search_scores_are_normalized_similarity` test checks `0.0 <= r.score <= 1.0`. After combined scoring, `score` is `final_score` which is still 0-1 (product of 0-1 values). This should still pass. If any tests fail, update their assertions to match the new scoring behavior.

- [ ] **Step 10: Run full test suite**

Run: `uv run pytest -v`
Expected: ALL PASS

- [ ] **Step 11: Commit**

```bash
git add src/qwick_memory/search.py tests/test_search.py
git commit -m "feat: cross-encoder reranking, threshold filtering, tuned hybrid weights"
```

---

## Tier 2: Auto-Ranking Feedback Loop

### Task 6: Wire retrieval count increment into search pipeline

**Files:**
- Modify: `src/qwick_memory/search.py`

- [ ] **Step 1: Add retrieval count increment to `search_memories`**

In `search_memories`, after step 3 (threshold filter) and before step 4 (combined scoring), add:

```python
  # Increment retrieval counts (fire-and-forget)
  try:
    from qwick_memory.stats import increment_retrieval
    increment_retrieval([r.id for r in results])
  except Exception:
    logger.debug("Failed to increment retrieval counts.")
```

- [ ] **Step 2: Run full test suite to verify nothing breaks**

Run: `uv run pytest -v`
Expected: ALL PASS

- [ ] **Step 3: Commit**

```bash
git add src/qwick_memory/search.py
git commit -m "feat: auto-increment retrieval counts on search"
```

---

### Task 7: MCP feedback tool + quality on save + PROTOCOL update

**Files:**
- Modify: `src/qwick_memory/server.py`
- Modify: `tests/test_server.py`

- [ ] **Step 1: Write failing test for feedback tool**

In `tests/test_server.py`, add:

```python
@pytest.mark.asyncio
async def test_qwick_memory_feedback(rag_env: str) -> None:
  """qwick_memory_feedback records usage stats."""
  import json
  from pathlib import Path

  from qwick_memory.server import qwick_memory_feedback, qwick_memory_save

  await qwick_memory_save("Memory for feedback test", repo="test/mcp-repo")
  result = await qwick_memory_feedback(used_ids="abc123", irrelevant_ids="def456")
  assert "Recorded" in result

  stats_path = Path(rag_env) / ".stats.json"
  stats = json.loads(stats_path.read_text())
  assert stats["abc123"]["usage_count"] == 1
```

- [ ] **Step 2: Run test to verify it fails**

Run: `uv run pytest tests/test_server.py::test_qwick_memory_feedback -v`
Expected: FAIL — `ImportError: cannot import name 'qwick_memory_feedback'`

- [ ] **Step 3: Add quality param to qwick_memory_save**

In `src/qwick_memory/server.py`, update the `qwick_memory_save` signature:

```python
@mcp.tool()
async def qwick_memory_save(
  content: str, type: str = "note", tags: str = "", repo: str = "", quality: int = 3
) -> str:
```

And pass `quality` to the `Memory` constructor:

```python
  memory = Memory(
    id=memory_id,
    repo=repo_list,
    type=type,
    tags=tag_list,
    author=author,
    created=datetime.now(timezone.utc),
    content=content,
    quality=max(1, min(5, quality)),
  )
```

Update the save tool docstring to include the quality rubric from the PROTOCOL draft.

- [ ] **Step 4: Add quality=3 to session_summary**

In `qwick_memory_session_summary`, pass `quality=3` to the `Memory` constructor.

- [ ] **Step 5: Implement qwick_memory_feedback tool**

```python
@mcp.tool()
async def qwick_memory_feedback(
  used_ids: str = "", irrelevant_ids: str = ""
) -> str:
  """Report which search results were useful after responding.

  Call this AFTER responding to a message where you used qwick_memory_search results.
  Only call once per response.

  Args:
    used_ids: Comma-separated memory IDs that you actually referenced in your response.
    irrelevant_ids: Comma-separated memory IDs that were noise/irrelevant.

  Returns:
    Confirmation string.
  """
  used = [i.strip() for i in used_ids.split(",") if i.strip()]
  irrelevant = [i.strip() for i in irrelevant_ids.split(",") if i.strip()]

  if not used and not irrelevant:
    return "No feedback provided."

  from qwick_memory.stats import record_feedback
  record_feedback(used_ids=used, irrelevant_ids=irrelevant)

  return (
    f"Recorded feedback: {len(used)} used, {len(irrelevant)} irrelevant.\n"
    f"-> This feedback improves future search ranking."
  )
```

- [ ] **Step 6: Update PROTOCOL constant**

Add the quality rubric to the save tool section and feedback instruction after the search section, as specified in the spec's PROTOCOL Updates draft section.

- [ ] **Step 7: Recalibrate tiered thresholds in server.py**

Update the display thresholds to match cross-encoder score ranges:

```python
HIGH_RELEVANCE_THRESHOLD = 0.6
MODERATE_RELEVANCE_THRESHOLD = 0.35
```

- [ ] **Step 8: Run server tests**

Run: `uv run pytest tests/test_server.py -v`
Expected: ALL PASS

- [ ] **Step 9: Commit**

```bash
git add src/qwick_memory/server.py tests/test_server.py
git commit -m "feat: feedback MCP tool, quality on save, PROTOCOL update"
```

---

### Task 8: Search interaction logging

**Files:**
- Modify: `src/qwick_memory/search.py`
- Modify: `src/qwick_memory/server.py`

- [ ] **Step 1: Implement `_log_search` in search.py**

```python
def _log_search(
  query: str,
  filters: dict[str, str | None],
  results: list[SearchResult],
  filtered_count: int,
) -> None:
  """Append search interaction to JSONL log. Fire-and-forget."""
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
        {"id": r.id, "reranker_score": round(r.reranker_score, 4), "final_score": round(r.score, 4)}
        for r in results
      ],
      "result_count": len(results),
      "filtered_count": filtered_count,
    }
    with open(log_path, "a") as f:
      f.write(json.dumps(entry) + "\n")
  except Exception:
    logger.debug("Failed to log search interaction.")
```

- [ ] **Step 2: Wire `_log_search` into `search_memories`**

At the end of `search_memories`, before `return results[:limit]`:

```python
  # Log search interaction (fire-and-forget)
  _log_search(
    query=query,
    filters={"repo_filter": repo, "type_filter": type_filter, "tag_filter": tag},
    results=results[:limit],
    filtered_count=pre_threshold_count - len(results),
  )
```

Track `pre_threshold_count = len(results)` before calling `_apply_thresholds`.

- [ ] **Step 3: Add feedback logging to server.py**

In `qwick_memory_feedback`, after `record_feedback(...)`, add:

```python
  try:
    from qwick_memory.config import get_search_log_path
    import json as _json

    log_path = get_search_log_path()
    entry = {
      "timestamp": datetime.now(timezone.utc).isoformat(),
      "type": "feedback",
      "used_ids": used,
      "irrelevant_ids": irrelevant,
    }
    with open(log_path, "a") as f:
      f.write(_json.dumps(entry) + "\n")
  except Exception:
    pass
```

- [ ] **Step 4: Run full test suite**

Run: `uv run pytest -v`
Expected: ALL PASS (logging is fire-and-forget, shouldn't affect test behavior)

- [ ] **Step 5: Commit**

```bash
git add src/qwick_memory/search.py src/qwick_memory/server.py
git commit -m "feat: search interaction logging to JSONL"
```

---

### Task 9: Update migrate command for schema version

**Files:**
- Modify: `src/qwick_memory/cli.py`

- [ ] **Step 1: Update migrate command to check schema_matches**

In `cli.py`, update the `migrate` command. After the model check block, add schema version check:

```python
  if not idx.model_matches():
    out.print("Model changed — rebuilding index...")
    stats = idx.build(memories_dir, force=True)
    out.print(f"Index rebuilt: {stats['new']} new. Total: {idx.count()}")
  elif not idx.schema_matches():
    out.print("Schema version changed — rebuilding index...")
    stats = idx.build(memories_dir, force=True)
    out.print(f"Index rebuilt: {stats['new']} new. Total: {idx.count()}")
  elif changed:
    stats = idx.build(memories_dir)
    out.print(f"Index updated: {stats['new']} new. Total: {idx.count()}")
```

- [ ] **Step 2: Run full test suite**

Run: `uv run pytest -v`
Expected: ALL PASS

- [ ] **Step 3: Commit**

```bash
git add src/qwick_memory/cli.py
git commit -m "feat: migrate command checks schema_version for auto-rebuild"
```

---

## Tier 3: CLI Updates + E2E Tests

### Task 10: CLI updates — quality flag, doctor, search display

**Files:**
- Modify: `src/qwick_memory/cli.py`
- Modify: `tests/test_cli.py`

- [ ] **Step 1: Add `--quality` option to save command**

```python
@app.command()
def save(
  content: str | None = typer.Argument(None, help="Memory content (opens $EDITOR if omitted)."),
  type: str = typer.Option("note", "--type", "-t", help="Memory type."),
  tags: str = typer.Option("", "--tags", help="Comma-separated tags."),
  repo: str = typer.Option(
    "", "--repo", "-r", help="Comma-separated repos (auto-detected if omitted)."
  ),
  quality: int = typer.Option(3, "--quality", "-q", help="Quality rating 1-5."),
  verbose: bool = verbose_option,
) -> None:
```

Pass `quality=max(1, min(5, quality))` to the `Memory` constructor.

- [ ] **Step 2: Add stats health check to doctor**

After the index consistency check in `doctor`, add:

```python
  # 7. Check stats file
  out.print("[bold]Checking stats file...[/bold]")
  stats_path = get_rag_dir() / ".stats.json"
  if stats_path.exists():
    try:
      import json
      stats = json.loads(stats_path.read_text())
      out.print(f"  Stats file: {len(stats)} entries")
    except Exception:
      console.print("  [yellow]Stats file is corrupted. Delete and it will regenerate.[/yellow]")
  else:
    out.print("  No stats file yet (created on first search feedback)")
```

- [ ] **Step 3: Run full test suite**

Run: `uv run pytest -v`
Expected: ALL PASS

- [ ] **Step 4: Commit**

```bash
git add src/qwick_memory/cli.py tests/test_cli.py
git commit -m "feat: CLI --quality flag on save, doctor checks stats"
```

---

### Task 11: Update existing tests for new pipeline

**Files:**
- Modify: `tests/test_search.py`
- Modify: `tests/test_server.py`

- [ ] **Step 1: Review and fix any broken existing tests**

**IMPORTANT:** The existing E2E test at `scripts/e2e-test.sh` ~line 150 has a comment saying irrelevant searches "should still return something." After threshold filtering, irrelevant searches now correctly return zero results. Update that E2E test section to expect "No results" output.

The new pipeline changes score distributions. Review each test in `test_search.py` and `test_server.py`:

- `test_search_returns_results` — should still pass (just checks `len > 0` and content)
- `test_search_with_repo_filter` — should still pass
- `test_search_with_type_filter` — should still pass
- `test_search_empty_index` — should still pass
- `test_search_result_has_score` — should still pass (score is now final_score, still > 0 for relevant results)
- `test_search_scores_are_normalized_similarity` — may need adjustment: `score` is now `final_score` (product of 0-1 values), still 0-1 range

For `test_server.py`:
- `test_qwick_memory_search` — should still pass (checks "PostgreSQL" in result text)

- [ ] **Step 2: Run full test suite, fix failures**

Run: `uv run pytest -v`
Fix any failures caused by the pipeline changes.

- [ ] **Step 3: Commit**

```bash
git add tests/
git commit -m "test: update existing tests for new search pipeline"
```

---

### Task 12: E2E test updates

**Files:**
- Modify: `scripts/e2e-test.sh`

- [ ] **Step 1: Add new E2E checks**

Add these checks to `scripts/e2e-test.sh`:

```bash
# -- Quality flag --
echo ""
echo -e "${BOLD}Testing quality flag...${RESET}"
OUTPUT=$(qwick-memory save "High quality memory with specific details" --type decision --tags "test" --quality 5 --repo test-repo 2>/dev/null)
EC=$?
assert_exit_code 0 $EC "save with --quality 5"
# Verify frontmatter contains quality
FILE=$(ls "$QWICK_MEMORY_DIR/memories/"*.md | head -1)
assert_contains "$(cat $FILE)" "quality: 5" "frontmatter contains quality: 5"

# -- Threshold filtering --
echo ""
echo -e "${BOLD}Testing threshold filtering...${RESET}"
OUTPUT=$(qwick-memory search "xyzzy blargh nonexistent gibberish" 2>/dev/null)
EC=$?
assert_exit_code 0 $EC "search irrelevant returns exit 0"
assert_contains "$OUTPUT" "No results" "irrelevant search returns no results"

# -- Schema version rebuild --
echo ""
echo -e "${BOLD}Testing schema version...${RESET}"
# Tamper with schema version in meta.json
META="$QWICK_MEMORY_DIR/.vectordb/meta.json"
if [ -f "$META" ]; then
  python3 -c "
import json, pathlib
p = pathlib.Path('$META')
m = json.loads(p.read_text())
m['schema_version'] = 0
p.write_text(json.dumps(m))
"
  OUTPUT=$(qwick-memory migrate 2>/dev/null)
  EC=$?
  assert_exit_code 0 $EC "migrate with schema mismatch"
  assert_contains "$OUTPUT" "Schema version changed" "migrate detects schema change"
fi
```

- [ ] **Step 2: Run E2E tests**

Run: `./scripts/e2e-test.sh`
Expected: ALL PASS

- [ ] **Step 3: Commit**

```bash
git add scripts/e2e-test.sh
git commit -m "test: add E2E checks for quality flag, thresholds, schema version"
```

---

### Task 13: Final integration verification

- [ ] **Step 1: Run full test suite**

Run: `uv run pytest -v`
Expected: ALL PASS

- [ ] **Step 2: Run E2E tests**

Run: `./scripts/e2e-test.sh`
Expected: ALL PASS

- [ ] **Step 3: Run ruff format + lint + pyright**

```bash
uv run ruff format src/ tests/
uv run ruff check src/ tests/
uv run pyright src/
```

Fix any issues.

- [ ] **Step 4: Test the original failing query**

```bash
uv run qwick-memory index --force
uv run qwick-memory search "cmux skill struggles issues problems"
```

Expected: 0-1 results. If the cmux memory exists, it should appear. No garbage results with 0.01 scores.

- [ ] **Step 5: Final commit with any formatting/lint fixes**

```bash
git add -A
git commit -m "style: formatting and lint fixes for search quality v2"
```

- [ ] **Step 6: Update CLAUDE.md**

Update `CLAUDE.md` with:
- New module `stats.py` in Module Map table
- MCP tool count: 7 → 8 (add `qwick_memory_feedback`)
- `quality` field in Memory Data Model section
- Updated save flow mentioning quality scoring

- [ ] **Step 7: Bump version in marketplace.json and plugin.json**

Update version from `"0.1.0"` to `"0.2.0"` in:
- `.claude-plugin/marketplace.json`
- `.claude-plugin/plugin.json`
- `pyproject.toml`

```bash
git add .claude-plugin/ pyproject.toml
git commit -m "chore: bump version to 0.2.0 for search quality v2"
```
