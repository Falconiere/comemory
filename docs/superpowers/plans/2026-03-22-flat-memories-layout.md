# Flat Memories Layout Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the `memories/{repo}/` subdirectory structure — all memory files go flat into `memories/{id}.md`. The repo reference lives in the YAML frontmatter only.

**Architecture:** Currently, saves create `memories/{repo}/{id}.md`. We flatten to `memories/{id}.md`. The `repo` field is already in every memory's frontmatter, so no data is lost. Search and filtering already use the parsed frontmatter, not the directory structure. The only code that relies on the repo subdirectory is: save paths (3 locations), session-summary rotation (1 location), and the e2e test delete step.

**Tech Stack:** Python (Typer CLI, FastMCP MCP server), pytest, bash (e2e script)

---

### Task 1: Flatten save path in CLI

**Files:**
- Modify: `src/qwick_memory/cli.py:90-94,112`

- [ ] **Step 1: Write the failing test**

The existing `test_save_creates_memory_file` already uses `rglob` so it passes either way. Add a test that asserts the file lands directly in `memories/`, not a subdirectory:

```python
# tests/test_cli.py — add after test_save_creates_memory_file

def test_save_creates_flat_file(tmp_path: Path) -> None:
  """save creates .md file directly in memories/, not in a repo subdirectory."""
  result = runner.invoke(app, ["save", "Flat layout test memory"])
  assert result.exit_code == 0, result.output
  md_files = list((tmp_path / "memories").glob("*.md"))
  assert len(md_files) == 1, f"Expected 1 file in memories/, got {md_files}"
  # No subdirectories should exist
  subdirs = [p for p in (tmp_path / "memories").iterdir() if p.is_dir()]
  assert subdirs == [], f"Unexpected subdirectories: {subdirs}"
```

- [ ] **Step 2: Run test to verify it fails**

Run: `uv run pytest tests/test_cli.py::test_save_creates_flat_file -v`
Expected: FAIL — file is created in `memories/test-repo/`, not `memories/`

- [ ] **Step 3: Implement — flatten save path in cli.py**

In `src/qwick_memory/cli.py`, replace lines 90-94 and update line 112:

```python
# OLD (lines 90-94):
  memories_dir = get_memories_dir()
  repo_dir = memories_dir / repo
  repo_dir.mkdir(parents=True, exist_ok=True)

  final_path = repo_dir / f"{memory_id}.md"

# NEW:
  memories_dir = get_memories_dir()
  memories_dir.mkdir(parents=True, exist_ok=True)

  final_path = memories_dir / f"{memory_id}.md"
```

```python
# OLD (line 112):
  tmp_path = repo_dir / f".{memory_id}.tmp"

# NEW:
  tmp_path = memories_dir / f".{memory_id}.tmp"
```

- [ ] **Step 4: Run test to verify it passes**

Run: `uv run pytest tests/test_cli.py -v`
Expected: ALL PASS

- [ ] **Step 5: Commit**

```bash
git add src/qwick_memory/cli.py tests/test_cli.py
git commit -m "refactor: flatten CLI save path — memories/{id}.md instead of memories/{repo}/{id}.md"
```

---

### Task 2: Flatten save path in MCP server `qwick_memory_save`

**Files:**
- Modify: `src/qwick_memory/server.py:100-104,122`

- [ ] **Step 1: Write the failing test**

```python
# tests/test_server.py — add after test_qwick_memory_save

@pytest.mark.asyncio
async def test_save_creates_flat_file(rag_env: str) -> None:
  """qwick_memory_save creates file directly in memories/, not a repo subdirectory."""
  from pathlib import Path

  from qwick_memory.server import qwick_memory_save

  await qwick_memory_save("Flat layout server test")
  memories_dir = Path(rag_env) / "memories"
  md_files = list(memories_dir.glob("*.md"))
  assert len(md_files) == 1, f"Expected 1 file in memories/, got {md_files}"
  subdirs = [p for p in memories_dir.iterdir() if p.is_dir()]
  assert subdirs == [], f"Unexpected subdirectories: {subdirs}"
```

- [ ] **Step 2: Run test to verify it fails**

Run: `uv run pytest tests/test_server.py::test_save_creates_flat_file -v`
Expected: FAIL — file goes to `memories/test/mcp-repo/`

- [ ] **Step 3: Implement — flatten save path in server.py qwick_memory_save**

In `src/qwick_memory/server.py`, replace lines 100-104 and update line 122:

```python
# OLD (lines 100-104):
  memories_dir = get_memories_dir()
  repo_dir = memories_dir / repo
  repo_dir.mkdir(parents=True, exist_ok=True)

  final_path = repo_dir / f"{memory_id}.md"

# NEW:
  memories_dir = get_memories_dir()
  memories_dir.mkdir(parents=True, exist_ok=True)

  final_path = memories_dir / f"{memory_id}.md"
```

```python
# OLD (line 122):
  tmp_path = repo_dir / f".{memory_id}.tmp"

# NEW:
  tmp_path = memories_dir / f".{memory_id}.tmp"
```

- [ ] **Step 4: Run test to verify it passes**

Run: `uv run pytest tests/test_server.py -v`
Expected: ALL PASS

- [ ] **Step 5: Commit**

```bash
git add src/qwick_memory/server.py tests/test_server.py
git commit -m "refactor: flatten MCP save path — qwick_memory_save writes to memories/{id}.md"
```

---

### Task 3: Flatten session-summary save + update rotation

**Files:**
- Modify: `src/qwick_memory/server.py:355-379,423-427,445,458`

This is the trickiest change. `_rotate_session_summaries` currently takes a `repo_dir` and globs only within that folder. After flattening, it must scan `memories_dir` and filter by repo from frontmatter.

- [ ] **Step 1: Write the failing test**

```python
# tests/test_server.py — add after test_qwick_memory_session_summary_rotation

@pytest.mark.asyncio
async def test_session_summary_creates_flat_file(rag_env: str) -> None:
  """qwick_memory_session_summary creates file directly in memories/."""
  from pathlib import Path

  from qwick_memory.server import qwick_memory_session_summary

  await qwick_memory_session_summary(
    goal="Flat test",
    discoveries="None",
    accomplished="Testing",
    next_steps="Verify",
    relevant_files="test.py",
  )
  memories_dir = Path(rag_env) / "memories"
  md_files = list(memories_dir.glob("*.md"))
  assert len(md_files) == 1, f"Expected 1 file in memories/, got {md_files}"
  subdirs = [p for p in memories_dir.iterdir() if p.is_dir()]
  assert subdirs == [], f"Unexpected subdirectories: {subdirs}"
```

- [ ] **Step 2: Run test to verify it fails**

Run: `uv run pytest tests/test_server.py::test_session_summary_creates_flat_file -v`
Expected: FAIL

- [ ] **Step 3: Implement — flatten session-summary save + update rotation**

In `src/qwick_memory/server.py`:

**Flatten `qwick_memory_session_summary` (lines 423-427, 445):**
```python
# OLD (lines 423-427):
  memories_dir = get_memories_dir()
  repo_dir = memories_dir / repo
  repo_dir.mkdir(parents=True, exist_ok=True)

  final_path = repo_dir / f"{memory_id}.md"

# NEW:
  memories_dir = get_memories_dir()
  memories_dir.mkdir(parents=True, exist_ok=True)

  final_path = memories_dir / f"{memory_id}.md"
```

```python
# OLD (line 445):
  tmp_path = repo_dir / f".{memory_id}.tmp"

# NEW:
  tmp_path = memories_dir / f".{memory_id}.tmp"
```

**Update `_rotate_session_summaries` signature and body (lines 355-379):**
```python
# OLD:
def _rotate_session_summaries(repo_dir: Path, max_keep: int = 3) -> None:
  """Delete old session summaries, keeping only the most recent `max_keep`."""
  summaries: list[tuple[datetime, Path]] = []
  for fp in repo_dir.glob("*.md"):
    try:
      mem = parse_memory(fp)
      if mem.type == "session-summary":
        summaries.append((mem.created, fp))
    except Exception:
      continue

# NEW:
def _rotate_session_summaries(memories_dir: Path, repo: str, max_keep: int = 3) -> None:
  """Delete old session summaries for a repo, keeping only the most recent `max_keep`."""
  summaries: list[tuple[datetime, Path]] = []
  for fp in memories_dir.glob("*.md"):
    try:
      mem = parse_memory(fp)
      if mem.type == "session-summary" and mem.repo == repo:
        summaries.append((mem.created, fp))
    except Exception:
      continue
```

**Update the call site (line 458):**
```python
# OLD:
    _rotate_session_summaries(repo_dir, max_keep=3)

# NEW:
    _rotate_session_summaries(memories_dir, repo, max_keep=3)
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `uv run pytest tests/test_server.py -v`
Expected: ALL PASS (including existing rotation test)

- [ ] **Step 5: Commit**

```bash
git add src/qwick_memory/server.py tests/test_server.py
git commit -m "refactor: flatten session-summary save + update rotation to filter by repo from frontmatter"
```

---

### Task 4: Simplify `scan_memories` (optional optimization)

**Files:**
- Modify: `src/qwick_memory/memory.py:127-129`

Now that there are no subdirectories, `rglob` is overkill. Simplify to `glob`.

- [ ] **Step 1: Run existing tests to confirm green baseline**

Run: `uv run pytest tests/ -v`
Expected: ALL PASS

- [ ] **Step 2: Change `rglob` to `glob` in scan_memories**

```python
# OLD:
def scan_memories(memories_dir: Path) -> list[Path]:
  """Return all markdown files found recursively under memories_dir."""
  return list(memories_dir.rglob("*.md"))

# NEW:
def scan_memories(memories_dir: Path) -> list[Path]:
  """Return all markdown files under memories_dir."""
  return list(memories_dir.glob("*.md"))
```

- [ ] **Step 3: Run tests to confirm nothing breaks**

Run: `uv run pytest tests/ -v`
Expected: ALL PASS

- [ ] **Step 4: Commit**

```bash
git add src/qwick_memory/memory.py
git commit -m "refactor: simplify scan_memories from rglob to glob (flat layout)"
```

---

### Task 5: Align git_utils tests with flat layout

**Files:**
- Modify: `tests/test_git_utils.py:67-68,95-97,132-133,139`

These tests create `memories/test-repo/` explicitly. Align with the new flat layout so test fixtures match production reality.

- [ ] **Step 1: Update test file paths**

```python
# test_git_sync_creates_orphan_branch_and_commits (line 67-68):
# OLD:
  memories = tmp_path / "memories" / "test-repo"
  memories.mkdir(parents=True)
  (memories / "abc123.md").write_text("test content")
# NEW:
  memories = tmp_path / "memories"
  memories.mkdir(parents=True)
  (memories / "abc123.md").write_text("test content")

# test_git_sync_creates_gitignore (lines 95-97):
# OLD:
  memories = tmp_path / "memories" / "test-repo"
  memories.mkdir(parents=True)
  (memories / "abc123.md").write_text("test content")
# NEW:
  memories = tmp_path / "memories"
  memories.mkdir(parents=True)
  (memories / "abc123.md").write_text("test content")

# test_git_sync_skips_setup_when_already_ready (lines 132-134, 139):
# OLD:
  memories = tmp_path / "memories" / "test-repo"
  memories.mkdir(parents=True)
  (memories / "first.md").write_text("first")
  ...
  (memories / "second.md").write_text("second")
# NEW:
  memories = tmp_path / "memories"
  memories.mkdir(parents=True)
  (memories / "first.md").write_text("first")
  ...
  (memories / "second.md").write_text("second")
```

- [ ] **Step 2: Run tests**

Run: `uv run pytest tests/test_git_utils.py -v`
Expected: ALL PASS

- [ ] **Step 3: Commit**

```bash
git add tests/test_git_utils.py
git commit -m "test: update git_utils tests for flat memories layout"
```

---

### Task 6: Update `delete` + test `rglob` to `glob`

**Files:**
- Modify: `src/qwick_memory/cli.py:229`
- Modify: `src/qwick_memory/server.py:248`
- Modify: `tests/test_server.py:104` (rotation test)
- Modify: `tests/test_integration.py:56`

Consistently replace `rglob` with `glob` across production and test code.

- [ ] **Step 1: Run delete tests to confirm green baseline**

Run: `uv run pytest tests/test_cli.py::test_delete_removes_memory tests/test_server.py::test_delete_response_confirms_both_layers -v`
Expected: ALL PASS

- [ ] **Step 2: Change rglob to glob in delete functions and test files**

```python
# cli.py line 229:
# OLD:
  matches = list(memories_dir.rglob(f"{memory_id}.md"))
# NEW:
  matches = list(memories_dir.glob(f"{memory_id}.md"))

# server.py line 248:
# OLD:
  matches = list(memories_dir.rglob(f"{memory_id}.md"))
# NEW:
  matches = list(memories_dir.glob(f"{memory_id}.md"))

# tests/test_server.py line 104 (rotation test):
# OLD:
  all_files = list(memories_dir.rglob("*.md"))
# NEW:
  all_files = list(memories_dir.glob("*.md"))

# tests/test_integration.py line 56:
# OLD:
  files = list((rag_dir / "memories").rglob("*.md"))
# NEW:
  files = list((rag_dir / "memories").glob("*.md"))
```

- [ ] **Step 3: Run tests**

Run: `uv run pytest tests/ -v`
Expected: ALL PASS

- [ ] **Step 4: Commit**

```bash
git add src/qwick_memory/cli.py src/qwick_memory/server.py tests/test_server.py tests/test_integration.py
git commit -m "refactor: simplify rglob to glob across delete and tests (flat layout)"
```

---

### Task 7: Update e2e test script

**Files:**
- Modify: `scripts/e2e-test.sh:175`

- [ ] **Step 1: Update the delete test to use flat path**

```bash
# OLD (line 175):
FIRST_FILE=$(ls "$TEST_DIR/memories/e2e-test-repo/"*.md | head -1)

# NEW:
FIRST_FILE=$(ls "$TEST_DIR/memories/"*.md | head -1)
```

- [ ] **Step 2: Run the e2e test**

Run: `./scripts/e2e-test.sh`
Expected: All 28 checks pass

- [ ] **Step 3: Commit**

```bash
git add scripts/e2e-test.sh
git commit -m "test: update e2e script for flat memories layout"
```

---

### Task 8: Update documentation (CLAUDE.md, README.md, integration test comment)

**Files:**
- Modify: `tests/test_integration.py:55`
- Modify: `CLAUDE.md`
- Modify: `README.md:33`

- [ ] **Step 1: Fix the stale comment in test_integration.py**

```python
# OLD (line 55):
  # Delete one memory — files live under memories/{repo}/

# NEW:
  # Delete one memory
```

- [ ] **Step 2: Update CLAUDE.md Save Flow and Data Model sections**

In the **Save Flow** section, remove references to `memories/{repo}/`:
- Step 2: `memories/.{id}.tmp` instead of `memories/{repo}/.{id}.tmp`
- Step 5: `memories/{id}.md` instead of `memories/{repo}/{id}.md`

In the **Architecture** bullet:
- **Source of truth:** `Markdown files with YAML frontmatter in memories/{id}.md`

- [ ] **Step 3: Update README.md**

```markdown
# OLD (line 33):
  -> Markdown file written to ~/.qwick-memory/memories/{repo}/{id}.md

# NEW:
  -> Markdown file written to ~/.qwick-memory/memories/{id}.md
```

- [ ] **Step 4: Run full test suite to verify everything works**

Run: `uv run pytest tests/ -v`
Expected: ALL PASS

- [ ] **Step 5: Commit**

```bash
git add tests/test_integration.py CLAUDE.md README.md
git commit -m "docs: update CLAUDE.md, README.md, and integration test comment for flat memories layout"
```

---

### Task 9: Final verification

- [ ] **Step 1: Run full pytest suite**

Run: `uv run pytest tests/ -v`
Expected: ALL PASS

- [ ] **Step 2: Run e2e test**

Run: `./scripts/e2e-test.sh`
Expected: All checks pass

- [ ] **Step 3: Run linter and type checker**

Run: `uv run ruff check src/ tests/ && uv run ruff format --check src/ tests/ && uv run pyright src/`
Expected: No errors
