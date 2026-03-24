# Search Quality, Token Optimization & Scale Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Upgrade embedding model, add token-aware result formatting, enforce repo-as-required, enforce flat layout, reinforce behavioral protocol, and add scale guardrails.

**Architecture:** Swap embedding model from MiniLM (384d/256tok) to nomic-Q (768d/8192tok), split embed methods for prefix handling, move meta.json writes to post-rebuild only, normalize search scores to 0-1, add tiered result formatting with token budgets in server.py, make repo required on save, remove FTS rebuild from upsert.

**Tech Stack:** Python 3.10+, fastembed (nomic-ai/nomic-embed-text-v1.5-Q), LanceDB, FastMCP, Typer, pytest

**Spec:** `docs/superpowers/specs/2026-03-23-search-quality-token-optimization-design.md`

---

### Task 1: Upgrade embedding model constant and split `_embed()` into document/query methods

**Files:**
- Modify: `src/qwick_memory/index.py:21` (MODEL_NAME constant)
- Modify: `src/qwick_memory/index.py:50-54` (split `_embed` into `_embed_documents` + `_embed_query`)
- Modify: `src/qwick_memory/index.py:99` (upsert calls `_embed_documents`)
- Modify: `src/qwick_memory/index.py:172-173` (full build calls `_embed_documents`)
- Modify: `src/qwick_memory/index.py:220,230` (incremental build calls `_embed_documents`)
- Test: `tests/test_index.py`

- [ ] **Step 1: Write failing test for `_embed_documents` prefix**

```python
# Add to tests/test_index.py
def test_embed_documents_adds_prefix(vectordb_dir: Path) -> None:
  """_embed_documents prepends 'search_document: ' prefix to texts."""
  idx = MemoryIndex(vectordb_dir)
  # Embed same text with and without prefix — vectors should differ
  doc_vecs = idx._embed_documents(["hello world"])
  query_vecs = idx._embed_query("hello world")
  assert doc_vecs[0] != query_vecs
```

- [ ] **Step 2: Run test to verify it fails**

Run: `uv run pytest tests/test_index.py::test_embed_documents_adds_prefix -v`
Expected: FAIL — `AttributeError: 'MemoryIndex' object has no attribute '_embed_documents'`

- [ ] **Step 3: Update MODEL_NAME and split `_embed` into two methods**

In `src/qwick_memory/index.py`:

```python
# Line 21: change model name
MODEL_NAME = "nomic-ai/nomic-embed-text-v1.5-Q"

# Lines 38: update docstring
@property
def model(self) -> TextEmbedding:
  """Lazy-load the embedding model (first call downloads ~130 MB)."""
  if self._model is None:
    self._model = TextEmbedding(MODEL_NAME)
  return self._model

# Replace _embed (lines 50-54) with two methods:
def _embed_documents(self, texts: list[str]) -> list[list[float]]:
  """Embed documents with 'search_document: ' prefix for nomic model."""
  if not texts:
    return []
  prefixed = [f"search_document: {t}" for t in texts]
  return [vec.tolist() for vec in self.model.embed(prefixed)]

def _embed_query(self, text: str) -> list[float]:
  """Embed a single query with 'search_query: ' prefix for nomic model."""
  prefixed = f"search_query: {text}"
  return list(self.model.embed([prefixed]))[0].tolist()
```

- [ ] **Step 4: Update all callers of `_embed` in `index.py`**

In `upsert()` (line 99): `self._embed([memory.content])` → `self._embed_documents([memory.content])`

In `_full_build()` (line 173): `self._embed(texts)` → `self._embed_documents(texts)`

In `_incremental_build()` (line 220): `self._embed([mem.content])` → `self._embed_documents([mem.content])`

In `_incremental_build()` (line 230): `self._embed(texts)` → `self._embed_documents(texts)`

- [ ] **Step 5: Run test to verify it passes**

Run: `uv run pytest tests/test_index.py::test_embed_documents_adds_prefix -v`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/qwick_memory/index.py tests/test_index.py
git commit -m "feat: upgrade to nomic-embed-text-v1.5-Q, split _embed into document/query methods"
```

---

### Task 2: Update `search.py` to use `_embed_query` and normalize scores

**Files:**
- Modify: `src/qwick_memory/search.py:46` (use `_embed_query` instead of `_embed`)
- Modify: `src/qwick_memory/search.py:104` (normalize `_distance` to similarity)
- Test: `tests/test_search.py`

- [ ] **Step 1: Write failing test for score normalization**

```python
# Add to tests/test_search.py
def test_search_scores_are_normalized_similarity(built_index: MemoryIndex) -> None:
  """All search scores should be in 0-1 range (normalized similarity)."""
  results = search_memories(built_index, "PostgreSQL database")
  assert len(results) > 0
  for r in results:
    assert 0.0 <= r.score <= 1.0, f"Score {r.score} not in 0-1 range"
```

- [ ] **Step 2: Run test to verify it fails**

Run: `uv run pytest tests/test_search.py::test_search_scores_are_normalized_similarity -v`
Expected: May FAIL if `_distance` scores are > 1.0 (L2 distance range is 0-2)

- [ ] **Step 3: Update `search.py` — use `_embed_query` and normalize scores**

```python
# Line 46: change _embed to _embed_query
query_vector = index._embed_query(query)

# Line 104: normalize _distance to similarity
# In _vector_search, change the return:
return [_row_to_result(row, score_key="_distance", normalize=True) for row in rows]

# Line 84: hybrid search keeps _relevance_score as-is
return [_row_to_result(row, score_key="_relevance_score", normalize=False) for row in rows]

# Update _row_to_result signature:
def _row_to_result(row: dict[str, Any], score_key: str, normalize: bool = False) -> SearchResult:
  """Map a LanceDB result row to a SearchResult dataclass."""
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
  )
```

- [ ] **Step 4: Run test to verify it passes**

Run: `uv run pytest tests/test_search.py -v`
Expected: ALL PASS

- [ ] **Step 5: Commit**

```bash
git add src/qwick_memory/search.py tests/test_search.py
git commit -m "feat: use _embed_query for search, normalize scores to 0-1 similarity"
```

---

### Task 3: Move `_write_meta()` out of `__init__()` and add `model_matches()`

**Files:**
- Modify: `src/qwick_memory/index.py:27-32` (`__init__` reads meta, doesn't write)
- Modify: `src/qwick_memory/index.py:45-48` (`_write_meta` unchanged but called later)
- Modify: `src/qwick_memory/index.py:70-79` (`_create_table` calls `_write_meta`)
- Modify: `src/qwick_memory/index.py:134-158` (`build` checks `model_matches`, calls `_write_meta` after)
- Test: `tests/test_index.py`

- [ ] **Step 1: Write failing test for meta.json not overwritten on init**

```python
# Add to tests/test_index.py
def test_meta_not_overwritten_on_init(tmp_path: Path) -> None:
  """MemoryIndex.__init__ should NOT overwrite meta.json with current model."""
  vectordb_dir = tmp_path / "vectordb"
  vectordb_dir.mkdir()
  meta_path = vectordb_dir / "meta.json"
  meta_path.write_text(json.dumps({"model": "old-model-name"}))

  MemoryIndex(vectordb_dir)

  meta = json.loads(meta_path.read_text())
  assert meta["model"] == "old-model-name", "init should not overwrite existing meta.json"
```

- [ ] **Step 2: Write failing test for model_matches**

```python
# Add to tests/test_index.py
def test_model_matches_detects_mismatch(tmp_path: Path) -> None:
  """model_matches() returns False when meta.json has a different model."""
  vectordb_dir = tmp_path / "vectordb"
  vectordb_dir.mkdir()
  meta_path = vectordb_dir / "meta.json"
  meta_path.write_text(json.dumps({"model": "old-model-name"}))

  idx = MemoryIndex(vectordb_dir)
  assert idx.model_matches() is False
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `uv run pytest tests/test_index.py::test_meta_not_overwritten_on_init tests/test_index.py::test_model_matches_detects_mismatch -v`
Expected: FAIL — meta.json overwritten by init, no `model_matches` method

- [ ] **Step 4: Implement changes in `index.py`**

```python
# In __init__ (lines 27-32): read meta instead of write
def __init__(self, vectordb_dir: Path) -> None:
  self._vectordb_dir = vectordb_dir
  self._vectordb_dir.mkdir(parents=True, exist_ok=True)
  self._db = lancedb.connect(str(vectordb_dir))
  self._model: TextEmbedding | None = None
  self._current_meta = self._read_meta()

# Add _read_meta method after __init__:
def _read_meta(self) -> dict[str, str]:
  """Read meta.json if it exists, return empty dict otherwise."""
  meta_path = self._vectordb_dir / "meta.json"
  if meta_path.exists():
    return json.loads(meta_path.read_text())
  return {}

# Add model_matches public method:
def model_matches(self) -> bool:
  """Check if the indexed model matches the current MODEL_NAME."""
  return self._current_meta.get("model") == MODEL_NAME

# _create_table stays unchanged (no _write_meta here — it's called in build() only)

# In build() (lines 134-158): auto-force when model mismatches, write meta ONCE after success
def build(self, memories_dir: Path, force: bool = False) -> dict[str, int]:
  md_files = scan_memories(memories_dir)
  # ... parse disk_memories (unchanged) ...

  if not self.model_matches() and not force:
    logger.info("Model changed (%s → %s). Forcing full rebuild.", self._current_meta.get("model", "none"), MODEL_NAME)
    force = True

  if force or not self._table_exists():
    stats = self._full_build(disk_memories)
  else:
    stats = self._incremental_build(disk_memories)

  # Write meta ONCE after any successful build path
  self._write_meta()
  self._current_meta = {"model": MODEL_NAME}
  return stats
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `uv run pytest tests/test_index.py -v`
Expected: ALL PASS

- [ ] **Step 6: Commit**

```bash
git add src/qwick_memory/index.py tests/test_index.py
git commit -m "feat: move _write_meta out of __init__, add model_matches() for migration detection"
```

---

### Task 4: Remove FTS rebuild from `upsert()`

**Files:**
- Modify: `src/qwick_memory/index.py:112-113` (remove FTS rebuild from upsert)
- Test: existing tests should still pass

- [ ] **Step 1: Remove FTS rebuild lines from `upsert()`**

In `src/qwick_memory/index.py`, remove lines 112-113:

```python
# REMOVE these two lines from upsert():
    with contextlib.suppress(Exception):
      table.create_fts_index("content", replace=True)
```

Keep the `optimize()` call on lines 114-115.

- [ ] **Step 2: Run all tests to verify nothing breaks**

Run: `uv run pytest -v`
Expected: ALL PASS (FTS is only needed for hybrid search, which rebuilds during `build()`)

- [ ] **Step 3: Commit**

```bash
git add src/qwick_memory/index.py
git commit -m "perf: remove FTS rebuild from upsert, only rebuild during build()"
```

---

### Task 5: Enforce flat memory layout

**Files:**
- Modify: `src/qwick_memory/memory.py:59-71` (validate flat path in `write_memory`)
- Modify: `src/qwick_memory/memory.py:135-137` (warn on subdirs in `scan_memories`)
- Test: `tests/test_memory.py`

- [ ] **Step 1: Write failing test for nested path rejection**

```python
# Add to tests/test_memory.py
from qwick_memory.errors import StorageError

def test_write_memory_rejects_nested_path(tmp_path: Path) -> None:
  """write_memory raises StorageError when target is in a subdirectory."""
  memories_dir = tmp_path / "memories"
  nested_dir = memories_dir / "0.1.0"
  nested_dir.mkdir(parents=True)
  mem = Memory(
    id="aabbccddeeff",
    repo=["test/repo"],
    type="note",
    tags=[],
    author="tester",
    created=datetime(2026, 1, 1),
    content="Should fail",
  )
  with pytest.raises(StorageError, match="nested"):
    write_memory(mem, nested_dir / "test.md", memories_dir=memories_dir)
```

**Note:** Also update all callers of `write_memory` in `server.py` and `cli.py` to pass `memories_dir=memories_dir`.

- [ ] **Step 2: Write failing test for scan_memories ignoring nested files**

```python
# Add to tests/test_memory.py
def test_scan_memories_ignores_nested_files(tmp_path: Path) -> None:
  """scan_memories only returns files directly in memories_dir, not subdirectories."""
  memories_dir = tmp_path / "memories"
  memories_dir.mkdir()
  # Create a file directly in memories/
  (memories_dir / "top.md").write_text("---\nid: top\n---\ntop level")
  # Create a nested file
  sub = memories_dir / "subdir"
  sub.mkdir()
  (sub / "nested.md").write_text("---\nid: nested\n---\nnested content")

  results = scan_memories(memories_dir)
  assert len(results) == 1
  assert results[0].name == "top.md"
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `uv run pytest tests/test_memory.py::test_write_memory_rejects_nested_path tests/test_memory.py::test_scan_memories_ignores_nested_files -v`
Expected: `test_write_memory_rejects_nested_path` FAILS (no StorageError raised). `test_scan_memories_ignores_nested_files` may PASS (glob already flat) — that's fine, it's a regression guard.

- [ ] **Step 4: Add flat layout validation to `write_memory` and warning to `scan_memories`**

In `src/qwick_memory/memory.py`:

```python
# Add import at top:
from qwick_memory.errors import MemoryParseError, StorageError

# Add logging:
import logging
logger = logging.getLogger(__name__)

# Add memories_dir parameter to write_memory for flat layout validation:
def write_memory(memory: Memory, filepath: Path, memories_dir: Path | None = None) -> None:
  """Serialize a Memory to a markdown file with YAML frontmatter."""
  # Enforce flat layout: filepath parent must match memories_dir exactly
  if memories_dir is not None and filepath.parent.resolve() != memories_dir.resolve():
    raise StorageError(
      f"Cannot write to nested path: {filepath}",
      suggested_fix="Write directly to the memories/ directory, not a subdirectory.",
      context={"filepath": str(filepath)},
    )
  # ... rest of existing function unchanged

# In scan_memories (after line 137):
def scan_memories(memories_dir: Path) -> list[Path]:
  """Return all markdown files directly in memories_dir (flat layout only)."""
  subdirs = [p for p in memories_dir.iterdir() if p.is_dir()] if memories_dir.exists() else []
  if subdirs:
    logger.warning(
      "Found subdirectories in memories/: %s. "
      "Flat layout expected — these will be ignored.",
      [d.name for d in subdirs],
    )
  return list(memories_dir.glob("*.md"))
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `uv run pytest tests/test_memory.py -v`
Expected: ALL PASS

- [ ] **Step 6: Commit**

```bash
git add src/qwick_memory/memory.py tests/test_memory.py
git commit -m "feat: enforce flat memory layout, reject nested paths, warn on subdirs"
```

---

### Task 6: Make `repo` required on save in MCP server

**Files:**
- Modify: `src/qwick_memory/server.py:65-66` (remove default for repo)
- Modify: `src/qwick_memory/server.py:85-87` (update tool description)
- Modify: `src/qwick_memory/server.py:103-114` (repo validation)
- Modify: `src/qwick_memory/server.py:410-411` (session summary repo param)
- Modify: `src/qwick_memory/server.py:444-453` (session summary repo validation)
- Test: `tests/test_server.py`

- [ ] **Step 1: Write failing test for save without repo**

```python
# Add to tests/test_server.py

@pytest.mark.asyncio
async def test_save_requires_repo(rag_env: str) -> None:
  """qwick_memory_save returns error when repo is empty string."""
  from qwick_memory.server import qwick_memory_save

  # repo="" should always error, regardless of env vars
  result = await qwick_memory_save("Test memory", repo="")
  assert "Error" in result
  assert "repo is required" in result
```

- [ ] **Step 2: Write failing test for save response confirming repos**

```python
@pytest.mark.asyncio
async def test_save_response_confirms_repo(rag_env: str) -> None:
  """qwick_memory_save response explicitly names the repos."""
  from qwick_memory.server import qwick_memory_save

  result = await qwick_memory_save(
    "Auth middleware decision", type="decision", repo="sidegig-api,sidegig-web"
  )
  assert "sidegig-api" in result
  assert "sidegig-web" in result
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `uv run pytest tests/test_server.py::test_save_requires_repo tests/test_server.py::test_save_response_confirms_repo -v`
Expected: FAIL — current code falls back to get_repo()

- [ ] **Step 4: Update `qwick_memory_save` — make repo required, update description**

In `src/qwick_memory/server.py`, modify `qwick_memory_save`:

```python
@mcp.tool()
async def qwick_memory_save(
  content: str, type: str = "note", tags: str = "", repo: str = ""
) -> str:
  """Save a memory to the knowledge base. Called proactively — do NOT wait for user to ask.

  CALL THIS TOOL AFTER:
  - Making an architecture, convention, or workflow decision
  - Fixing a bug (include root cause and fix)
  - Discovering a non-obvious gotcha or edge case
  - Establishing a pattern or convention
  - Learning a user preference or constraint
  - Implementing a feature with a non-obvious approach

  Self-check after every task: "Did I just decide, fix, learn, or
  establish something?" If yes → save NOW.

  Args:
    content: The memory content to save.
    type: Memory type (decision, bug, convention, discovery, pattern, preference, note).
    tags: Comma-separated tags for discoverability.
    repo: Comma-separated repo names (e.g. 'qwick-mobile' or 'sidegig-api,sidegig-web').
          REQUIRED — always specify which repo(s) this memory belongs to. Never omit.

  Returns:
    Status string confirming the save with indexing details.
  """
  if not content or not content.strip():
    return "Error: content cannot be empty."

  content = content.strip()

  if type not in MEMORY_TYPES:
    return f"Error: Invalid type '{type}'. Must be one of: {', '.join(MEMORY_TYPES)}"

  # Repo is required — no auto-detection fallback
  if not repo or not repo.strip():
    return (
      "Error: repo is required. Specify which repo(s) this memory belongs to "
      "(e.g. repo='sidegig-api' or repo='sidegig-api,sidegig-web')."
    )
  repo_list = [r.strip() for r in repo.split(",") if r.strip()]

  # ... rest unchanged from line 116 onward (author, memories_dir, etc.)
```

- [ ] **Step 5: Update `qwick_memory_session_summary` — same repo-required change**

```python
# In session summary function, replace repo detection block (lines 444-453):
  if not repo or not repo.strip():
    return (
      "Error: repo is required. Specify which repo(s) this session summary belongs to "
      "(e.g. repo='sidegig-api' or repo='sidegig-api,sidegig-web')."
    )
  repo_list = [r.strip() for r in repo.split(",") if r.strip()]
```

Also update the session summary tool description for repo param:

```python
    repo: Comma-separated repo names. REQUIRED — always specify which repo(s).
```

- [ ] **Step 6: Update ALL existing tests that call save/session_summary without explicit repo**

The server no longer calls `get_repo()` on save. Every call without `repo=` will fail. Add `repo="test/mcp-repo"` to ALL these call sites:

**`qwick_memory_save` calls (8 total):**
1. `test_qwick_memory_save` (line 29): add `repo="test/mcp-repo"`
2. `test_save_creates_flat_file` (line 40): add `repo="test/mcp-repo"`
3. `test_qwick_memory_search` (line 53): add `repo="test/mcp-repo"`
4. `test_save_response_includes_vector_hint` (line 189): add `repo="test/mcp-repo"`
5. `test_save_duplicate_response_hint` (lines 199-200): add `repo="test/mcp-repo"` to BOTH calls
6. `test_search_results_include_similarity_hint` (line 210): add `repo="test/mcp-repo"`
7. `test_delete_response_confirms_both_layers` (line 238): add `repo="test/mcp-repo"`
8. `test_qwick_memory_context_shows_summary_first` (line 157): add `repo="test/mcp-repo"`

**`qwick_memory_session_summary` calls (6 total):**
1. `test_qwick_memory_session_summary` (line 72): add `repo="test/mcp-repo"`
2. `test_qwick_memory_session_summary_empty_goal` (line 87): add `repo="test/mcp-repo"`
3. `test_qwick_memory_session_summary_rotation` (line 107): add `repo="test/mcp-repo"`
4. `test_session_summary_creates_flat_file` (line 134): add `repo="test/mcp-repo"`
5. `test_qwick_memory_context_shows_summary_first` (line 158): add `repo="test/mcp-repo"`
6. `test_session_summary_response_includes_vector_hint` (line 251): add `repo="test/mcp-repo"`

- [ ] **Step 7: Run all server tests**

Run: `uv run pytest tests/test_server.py -v`
Expected: ALL PASS

- [ ] **Step 8: Commit**

```bash
git add src/qwick_memory/server.py tests/test_server.py
git commit -m "feat: make repo required on save and session_summary, no auto-detection fallback"
```

---

### Task 7: Add token-aware tiered formatting to search results

**Files:**
- Modify: `src/qwick_memory/server.py:159-210` (rewrite `qwick_memory_search` response formatting)
- Test: `tests/test_server.py`

- [ ] **Step 1: Write failing test for tiered output**

```python
# Add to tests/test_server.py

@pytest.mark.asyncio
async def test_search_returns_tiered_format(rag_env: str) -> None:
  """qwick_memory_search returns structured tiered markdown output."""
  from qwick_memory.server import qwick_memory_save, qwick_memory_search

  await qwick_memory_save(
    "PostgreSQL is great for JSONB queries and relational data",
    repo="test/mcp-repo",
  )
  result = await qwick_memory_search("PostgreSQL JSONB")
  # Should contain tiered structure (at minimum one tier header or result line)
  assert "PostgreSQL" in result
  # Should contain the result count hint
  assert "result" in result.lower()
```

- [ ] **Step 2: Run test to verify current behavior**

Run: `uv run pytest tests/test_server.py::test_search_returns_tiered_format -v`

- [ ] **Step 3: Add formatting constants and helper function to `server.py`**

Add these constants near the top of `server.py` (after `PROTOCOL`):

```python
# Token budget and tier thresholds for search results
SEARCH_TOKEN_BUDGET = 4000
CONTEXT_TOKEN_BUDGET = 6000
CONTEXT_SUMMARY_BUDGET = 2000
HIGH_RELEVANCE_THRESHOLD = 0.7
MODERATE_RELEVANCE_THRESHOLD = 0.4


def _estimate_tokens(text: str) -> int:
  """Rough token estimate: ~4 chars per token for English text."""
  return len(text) // 4


def _format_tiered_results(results: list, budget: int = SEARCH_TOKEN_BUDGET) -> str:
  """Format search results into tiered markdown with token budget."""
  from qwick_memory.search import SearchResult

  high: list[SearchResult] = []
  moderate: list[SearchResult] = []
  low: list[SearchResult] = []

  for r in results:
    if r.score > HIGH_RELEVANCE_THRESHOLD:
      high.append(r)
    elif r.score > MODERATE_RELEVANCE_THRESHOLD:
      moderate.append(r)
    else:
      low.append(r)

  lines: list[str] = []
  remaining = budget

  # High relevance: full content
  if high:
    lines.append("### High Relevance")
    for r in high:
      repos = r.repo.replace(",", ", ")
      # Use first sentence (up to first period/newline) as title, max 80 chars
      first_sentence = r.content.split("\n")[0].split(".")[0][:80]
      header = f"**[{r.type}] {first_sentence}** — {repos} (tags: {r.tags})"
      entry = f"{header}\n{r.content}"
      cost = _estimate_tokens(entry)
      if cost > remaining:
        # Truncate to fit
        max_chars = remaining * 4
        entry = f"{header}\n{r.content[:max_chars]}... [truncated]"
        remaining = 0
      else:
        remaining -= cost
      lines.append(entry)
      lines.append("")
      if remaining <= 0:
        break

  # Moderate relevance: truncated
  if moderate and remaining > 0:
    lines.append("### Moderate Relevance")
    for r in moderate:
      repos = r.repo.replace(",", ", ")
      entry = f"**[{r.type}]** — {repos} | {r.content[:200]}... → ID: {r.id}"
      cost = _estimate_tokens(entry)
      if cost > remaining:
        break
      remaining -= cost
      lines.append(entry)
    lines.append("")

  # Low relevance: one-liners
  if low and remaining > 0:
    lines.append("### Low Relevance")
    for r in low:
      repos = r.repo.replace(",", ", ")
      first_sentence = r.content.split(".")[0][:80]
      entry = f"- [{r.type}] {first_sentence} — {repos} → ID: {r.id}"
      cost = _estimate_tokens(entry)
      if cost > remaining:
        break
      remaining -= cost
      lines.append(entry)

  return "\n".join(lines)
```

- [ ] **Step 4: Update `qwick_memory_search` to use tiered formatting**

Replace the result formatting block (lines 202-210):

```python
  result_text = _format_tiered_results(results)
  count = len(results)
  return (
    f"{count} result(s) found. Use these to inform your response — do NOT ignore them.\n\n"
    f"{result_text}"
  )
```

- [ ] **Step 5: Update existing search tests that check old format**

Tests like `test_search_results_include_similarity_hint` check for `"Results ranked by semantic similarity"` — update to check for the new format: `"result(s) found"`.

Tests like `test_qwick_memory_search` check for `"PostgreSQL" in result` — this should still pass since content is still included.

- [ ] **Step 6: Run all server tests**

Run: `uv run pytest tests/test_server.py -v`
Expected: ALL PASS

- [ ] **Step 7: Commit**

```bash
git add src/qwick_memory/server.py tests/test_server.py
git commit -m "feat: add token-aware tiered result formatting for search"
```

---

### Task 8: Add token budget to context tool

**Files:**
- Modify: `src/qwick_memory/server.py:311-373` (context tool with budget)
- Test: `tests/test_server.py`

- [ ] **Step 1: Write failing test for context budget**

```python
@pytest.mark.asyncio
async def test_context_respects_token_budget(rag_env: str) -> None:
  """qwick_memory_context output stays within CONTEXT_TOKEN_BUDGET."""
  from qwick_memory.server import CONTEXT_TOKEN_BUDGET, qwick_memory_context, qwick_memory_save

  # Save many memories to exceed budget
  for i in range(15):
    await qwick_memory_save(
      f"Memory number {i} with some detailed content about topic {i} " * 10,
      type="decision",
      tags=f"tag{i}",
      repo="test/mcp-repo",
    )

  result = await qwick_memory_context()
  estimated_tokens = len(result) // 4
  assert estimated_tokens <= CONTEXT_TOKEN_BUDGET * 1.2, (
    f"Context output ({estimated_tokens} tokens) exceeds budget ({CONTEXT_TOKEN_BUDGET})"
  )
```

- [ ] **Step 2: Run test to verify it fails**

Run: `uv run pytest tests/test_server.py::test_context_respects_token_budget -v`
Expected: May FAIL if output exceeds budget

- [ ] **Step 3: Update `qwick_memory_context` to use token budget**

Rewrite the formatting section of `qwick_memory_context`:

```python
  lines: list[str] = []
  remaining = CONTEXT_TOKEN_BUDGET

  # Section 1: Latest session summary (up to CONTEXT_SUMMARY_BUDGET tokens)
  if summaries:
    summaries.sort(key=lambda m: m.created, reverse=True)
    latest = summaries[0]
    lines.append("### Last Session")
    summary_text = latest.content
    summary_tokens = _estimate_tokens(summary_text)
    if summary_tokens > CONTEXT_SUMMARY_BUDGET:
      max_chars = CONTEXT_SUMMARY_BUDGET * 4
      summary_text = summary_text[:max_chars] + "\n\n[truncated]"
      remaining -= CONTEXT_SUMMARY_BUDGET
    else:
      remaining -= summary_tokens
    lines.append(summary_text)
    lines.append("")

  # Section 2: Recent non-summary memories (fill remaining budget)
  if regular:
    regular.sort(key=lambda m: m.created, reverse=True)
    lines.append("### Recent Memories")
    for mem in regular[:limit]:
      if remaining <= 0:
        break
      preview = mem.content[:120] + "..." if len(mem.content) > 120 else mem.content
      entry = f"- [{mem.created.isoformat()}] ({mem.type}) {preview}"
      cost = _estimate_tokens(entry)
      if cost > remaining:
        break
      remaining -= cost
      lines.append(entry)

  return "\n".join(lines)
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `uv run pytest tests/test_server.py -v`
Expected: ALL PASS

- [ ] **Step 5: Commit**

```bash
git add src/qwick_memory/server.py tests/test_server.py
git commit -m "feat: add token budget to context tool, prioritize session summary"
```

---

### Task 9: Strengthen protocol and tool descriptions

**Files:**
- Modify: `src/qwick_memory/server.py:27-59` (PROTOCOL constant)
- Modify: `src/qwick_memory/server.py:68,85-87` (save tool description)
- Modify: `src/qwick_memory/server.py:167-177` (search tool description)
- Modify: `src/qwick_memory/server.py:195-200` (search empty response)
- Modify: `scripts/session-start.sh` (add reminder)

- [ ] **Step 1: Update `qwick_memory_search` description**

Add to the docstring after `"-> Always search BEFORE answering from general knowledge."`:

```
  If you're about to answer from general knowledge, STOP — search first.
  Memory has project-specific context you don't.
```

- [ ] **Step 2: Update search empty response**

Replace the no-results response:

```python
  if not results:
    return (
      "No results found.\n"
      "-> If you learn something new about this topic during this task, "
      "save it before the session ends."
    )
```

- [ ] **Step 3: Update `scripts/session-start.sh` — add reminder**

Add a line at the end of the context output:

```bash
echo ""
echo "REMINDER: save decisions, bugs, and discoveries to qwick-memory. Always specify repo."
```

- [ ] **Step 4: Update `test_search_no_results_includes_save_hint`**

The test at line 221 checks for `"save it with qwick_memory_save"`. The new message says `"save it before the session ends"`. Update the assertion:

```python
assert "save it before the session ends" in result
```

- [ ] **Step 5: Run tests to verify nothing breaks**

Run: `uv run pytest -v`
Expected: ALL PASS.

- [ ] **Step 6: Commit**

```bash
git add src/qwick_memory/server.py scripts/session-start.sh tests/test_server.py
git commit -m "feat: strengthen behavioral protocol, sharper tool descriptions and response nudges"
```

---

### Task 10: Update CLI `TOKEN_WARN_LIMIT` and doctor nested-dir check

**Files:**
- Modify: `src/qwick_memory/cli.py:39` (update TOKEN_WARN_LIMIT)
- Modify: `src/qwick_memory/cli.py:329-437` (doctor command: add nested dir check, use model_matches)

- [ ] **Step 1: Update TOKEN_WARN_LIMIT**

In `src/qwick_memory/cli.py` line 39:

```python
TOKEN_WARN_LIMIT = 6000  # calibrated for nomic 8192-token context
```

- [ ] **Step 2: Add nested directory check to doctor**

After the "Check memory files validity" section (around line 362), add:

```python
  # 2b. Check for nested directories in memories/
  if memories_dir.exists():
    subdirs = [p for p in memories_dir.iterdir() if p.is_dir()]
    if subdirs:
      console.print(
        f"  [yellow]Found nested directories: {[d.name for d in subdirs]}. "
        f"Flat layout expected — remove them or move files to memories/.[/yellow]"
      )
  ```

- [ ] **Step 3: Update model version check in doctor to use model_matches**

In the doctor command's model version section (around line 398), the code already checks `meta.json` model vs `MODEL_NAME`. This still works correctly since `MemoryIndex.__init__` no longer overwrites meta.json. No change needed here — just verify it works.

- [ ] **Step 4: Run doctor and full tests**

Run: `uv run pytest tests/ -v && uv run qwick-memory doctor`
Expected: ALL PASS

- [ ] **Step 5: Commit**

```bash
git add src/qwick_memory/cli.py
git commit -m "feat: update TOKEN_WARN_LIMIT for nomic, add nested-dir check to doctor"
```

---

### Task 11: Update e2e test script

**Files:**
- Modify: `scripts/e2e-test.sh`

- [ ] **Step 1: Update search assertions for new tiered format**

In the e2e script section 3 (Search memories), update the assertions. The search output now uses tiered markdown instead of flat `[score] repo (type)` format. Update the `assert_contains` checks to match the new format — they check for content strings like "PostgreSQL" and "token" which will still appear in the tiered output.

No changes needed for content-based assertions. But add a new check:

```bash
# Add after existing search checks:
OUT=$($QR search "which database do we use" 2>&1) || true
assert_contains "$OUT" "result" "search output includes result count"
```

- [ ] **Step 2: Run the e2e test**

Run: `./scripts/e2e-test.sh`
Expected: ALL checks pass

- [ ] **Step 3: Commit**

```bash
git add scripts/e2e-test.sh
git commit -m "test: update e2e script for tiered search output format"
```

---

### Task 12: Update documentation

**Files:**
- Modify: `CLAUDE.md`
- Modify: `README.md`

- [ ] **Step 1: Update CLAUDE.md**

Update these sections:
- **Architecture / Embeddings** line: change model reference from `all-MiniLM-L6-v2` and `~30MB` to `nomic-ai/nomic-embed-text-v1.5-Q` and `~130MB`
- **Key Commands** section: add note about `qwick-memory index --force` needed after model upgrade
- **Testing** section: update first-run download note from `~30MB` to `~130MB`

- [ ] **Step 2: Update README.md**

Update any references to the embedding model name and size. Note the first-run download is ~130MB.

- [ ] **Step 3: Commit**

```bash
git add CLAUDE.md README.md
git commit -m "docs: update CLAUDE.md and README.md for nomic embedding model and repo-required change"
```

---

### Task 13: Run full test suite and e2e

**Files:** None (verification only)

- [ ] **Step 1: Run unit/integration tests**

Run: `uv run pytest -v`
Expected: ALL PASS

- [ ] **Step 2: Run e2e tests**

Run: `./scripts/e2e-test.sh`
Expected: ALL checks pass

- [ ] **Step 3: Run type checker**

Run: `uv run pyright src/`
Expected: No errors

- [ ] **Step 4: Run linter and formatter**

Run: `uv run ruff check src/ tests/ && uv run ruff format --check src/ tests/`
Expected: Clean

- [ ] **Step 5: Run doctor**

Run: `uv run qwick-memory doctor`
Expected: All checks pass (may show model mismatch until `index --force` is run on real data)

- [ ] **Step 6: Rebuild index with new model**

Run: `uv run qwick-memory index --force`
Expected: Full rebuild with nomic embeddings
