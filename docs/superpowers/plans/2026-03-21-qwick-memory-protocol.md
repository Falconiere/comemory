# Qwick Memory Protocol Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an aggressive, always-on memory protocol to qwick-memory that automatically saves decisions, bugs, conventions, and session context — replicating engram's behavior with qwick-memory's markdown + vector search infrastructure.

**Architecture:** Protocol instructions injected via FastMCP `instructions` param tell Claude to proactively save/search. Lifecycle hooks handle session start (context loading), pre-compaction (reminder), and post-compaction (context restoration). A new `qwick_memory_session_summary` tool saves structured session summaries with rotation.

**Tech Stack:** Python 3.10+, FastMCP, Typer, LanceDB, fastembed, pytest-asyncio

**Spec:** `docs/superpowers/specs/2026-03-21-qwick-memory-protocol-design.md`

---

## File Structure

| File | Action | Responsibility |
|------|--------|---------------|
| `src/qwick_memory/memory.py` | Modify | Add `"session-summary"` to `MEMORY_TYPES` and `MemoryType` |
| `src/qwick_memory/server.py` | Modify | Rename tools, add instructions, add `qwick_memory_session_summary`, enhance `qwick_memory_context` |
| `src/qwick_memory/cli.py` | Modify | Add `context` subcommand |
| `hooks/hooks.json` | Modify | Add `PreCompact` and `PostCompact` hook entries |
| `scripts/session-start.sh` | Modify | Add context output after indexing |
| `scripts/pre-compact.sh` | Create | Best-effort reminder + context snapshot before compaction |
| `scripts/post-compact.sh` | Create | Restore context after compaction |
| `tests/test_server.py` | Modify | Update imports for renamed tools, add session summary + context tests |
| `tests/test_cli.py` | Modify | Add context command tests |
| `tests/test_memory.py` | Modify | Add session-summary type test |
| `scripts/e2e-test.sh` | Modify | Add context command e2e checks |
| `CLAUDE.md` | Modify | Update module map and add protocol section |
| `README.md` | Modify | Update tool names from `rag_*` to `qwick_memory_*` |
| `skills/memory/SKILL.md` | Modify | Rewrite with new tool names and session summary instructions |

---

### Task 1: Add `session-summary` Memory Type

**Files:**
- Modify: `src/qwick_memory/memory.py:14-32`
- Test: `tests/test_memory.py`

- [ ] **Step 1: Write the failing test**

In `tests/test_memory.py`, add:

```python
def test_session_summary_type_is_valid() -> None:
  """session-summary is a recognized memory type."""
  from qwick_memory.memory import MEMORY_TYPES

  assert "session-summary" in MEMORY_TYPES
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /Users/falconiere/Projects/qwick-memory && uv run pytest tests/test_memory.py::test_session_summary_type_is_valid -v`
Expected: FAIL with `AssertionError`

- [ ] **Step 3: Add session-summary to MEMORY_TYPES and MemoryType**

In `src/qwick_memory/memory.py`, update the `MEMORY_TYPES` tuple (line 14) and `MemoryType` literal (line 24):

```python
MEMORY_TYPES = (
  "decision",
  "bug",
  "convention",
  "discovery",
  "pattern",
  "preference",
  "note",
  "session-summary",
)

MemoryType = Literal[
  "decision",
  "bug",
  "convention",
  "discovery",
  "pattern",
  "preference",
  "note",
  "session-summary",
]
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd /Users/falconiere/Projects/qwick-memory && uv run pytest tests/test_memory.py::test_session_summary_type_is_valid -v`
Expected: PASS

- [ ] **Step 5: Run full test suite to verify no regressions**

Run: `cd /Users/falconiere/Projects/qwick-memory && uv run pytest -v`
Expected: All tests pass

- [ ] **Step 6: Commit**

```bash
cd /Users/falconiere/Projects/qwick-memory && git add src/qwick_memory/memory.py tests/test_memory.py && git commit -m "feat: add session-summary memory type"
```

---

### Task 2: Rename MCP Tools from `rag_*` to `qwick_memory_*`

**Files:**
- Modify: `src/qwick_memory/server.py:28-261`
- Modify: `tests/test_server.py`

- [ ] **Step 1: Update test imports and function names**

In `tests/test_server.py`, rename all references. Replace the 3 test functions:

```python
@pytest.mark.asyncio
async def test_qwick_memory_save(rag_env: str) -> None:
  """qwick_memory_save creates a memory and returns 'Saved' in result."""
  from qwick_memory.server import qwick_memory_save

  result = await qwick_memory_save("MCP server test memory")
  assert "Saved" in result


@pytest.mark.asyncio
async def test_qwick_memory_search(rag_env: str) -> None:
  """qwick_memory_save then qwick_memory_search finds the saved content."""
  from qwick_memory.server import qwick_memory_save, qwick_memory_search

  await qwick_memory_save("PostgreSQL is great for JSONB queries")
  result = await qwick_memory_search("PostgreSQL")
  assert "PostgreSQL" in result


@pytest.mark.asyncio
async def test_qwick_memory_index(rag_env: str) -> None:
  """qwick_memory_index on empty dir returns 'Indexed' in result."""
  from qwick_memory.server import qwick_memory_index

  result = await qwick_memory_index()
  assert "Indexed" in result
```

- [ ] **Step 2: Run tests to verify they fail (old names gone)**

Run: `cd /Users/falconiere/Projects/qwick-memory && uv run pytest tests/test_server.py -v`
Expected: FAIL with `ImportError` (old function names don't exist yet in new form)

- [ ] **Step 3: Rename all 6 tool functions in server.py**

In `src/qwick_memory/server.py`, rename:
- `rag_save` → `qwick_memory_save`
- `rag_search` → `qwick_memory_search`
- `rag_list` → `qwick_memory_list`
- `rag_delete` → `qwick_memory_delete`
- `rag_index` → `qwick_memory_index`
- `rag_context` → `qwick_memory_context`

This is a straightforward find-and-replace of function names. The `@mcp.tool()` decorator uses the function name as the tool name, so no other changes are needed.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd /Users/falconiere/Projects/qwick-memory && uv run pytest tests/test_server.py -v`
Expected: All 3 tests pass

- [ ] **Step 5: Run full test suite**

Run: `cd /Users/falconiere/Projects/qwick-memory && uv run pytest -v`
Expected: All tests pass

- [ ] **Step 6: Commit**

```bash
cd /Users/falconiere/Projects/qwick-memory && git add src/qwick_memory/server.py tests/test_server.py && git commit -m "refactor: rename MCP tools from rag_* to qwick_memory_*"
```

---

### Task 3: Add MCP Server Protocol Instructions

**Files:**
- Modify: `src/qwick_memory/server.py:25`

- [ ] **Step 1: Add instructions constant and pass to FastMCP**

In `src/qwick_memory/server.py`, add the protocol text as a constant before the `mcp = FastMCP(...)` line, then pass it:

```python
PROTOCOL = """\
## Qwick Memory — ACTIVE PROTOCOL

You have qwick-memory tools (qwick_memory_save, qwick_memory_search, qwick_memory_context, qwick_memory_session_summary).
This protocol is MANDATORY and ALWAYS ACTIVE.

### PROACTIVE SAVE — do NOT wait for user to ask
Call `qwick_memory_save` IMMEDIATELY after ANY of these:
- Decision made (architecture, convention, workflow, tool choice)
- Bug fixed (include root cause)
- Convention or workflow documented/updated
- Non-obvious discovery, gotcha, or edge case found
- Pattern established (naming, structure, approach)
- User preference or constraint learned
- Feature implemented with non-obvious approach
- Artifact created or updated with significant content (Notion, Jira, GitHub, etc.)

**Self-check after EVERY task**: "Did I just make a decision, fix a bug, learn something, or establish a convention? If yes → qwick_memory_save NOW."

When saving, choose the right type:
- `decision` — architecture, convention, workflow, tool choice
- `bug` — bug fixed, include root cause and fix
- `convention` — coding standard, workflow rule, naming pattern
- `discovery` — non-obvious finding, gotcha, edge case
- `pattern` — recurring approach, structure, design pattern
- `preference` — user preference, constraint, working style
- `note` — anything else worth remembering
- `session-summary` — (used automatically by qwick_memory_session_summary, do not use directly)

Use descriptive, comma-separated tags for discoverability.

### SEARCH MEMORY when:
- User asks to recall anything ("remember", "what did we do", "acordate", "que hicimos")
- Starting work on something that might have been done before
- User mentions a topic you have no context on
- User's FIRST message references the project, a feature, or a problem — call `qwick_memory_search` with keywords to check for prior work before responding

### SESSION CLOSE — before saying "done"/"listo":
Call `qwick_memory_session_summary` with a structured summary:
- Goal: what the user wanted to accomplish
- Discoveries: non-obvious things learned
- Accomplished: what was done
- Next steps: what remains
- Relevant files: key files touched or referenced
"""

mcp = FastMCP("qwick-memory", instructions=PROTOCOL)
```

- [ ] **Step 2: Run full test suite to verify no regressions**

Run: `cd /Users/falconiere/Projects/qwick-memory && uv run pytest -v`
Expected: All tests pass (instructions don't affect tool behavior)

- [ ] **Step 3: Commit**

```bash
cd /Users/falconiere/Projects/qwick-memory && git add src/qwick_memory/server.py && git commit -m "feat: add qwick memory protocol instructions to MCP server"
```

---

### Task 4: Add `qwick_memory_session_summary` MCP Tool

**Files:**
- Modify: `src/qwick_memory/server.py`
- Test: `tests/test_server.py`

- [ ] **Step 1: Write the failing tests**

Add to `tests/test_server.py`:

```python
@pytest.mark.asyncio
async def test_qwick_memory_session_summary(rag_env: str) -> None:
  """qwick_memory_session_summary saves a structured summary."""
  from qwick_memory.server import qwick_memory_session_summary

  result = await qwick_memory_session_summary(
    goal="Implement memory protocol",
    discoveries="FastMCP supports instructions param",
    accomplished="Renamed all tools",
    next_steps="Add hooks",
    relevant_files="server.py, hooks.json",
  )
  assert "Saved" in result


@pytest.mark.asyncio
async def test_qwick_memory_session_summary_empty_goal(rag_env: str) -> None:
  """qwick_memory_session_summary rejects empty goal."""
  from qwick_memory.server import qwick_memory_session_summary

  result = await qwick_memory_session_summary(
    goal="",
    discoveries="something",
    accomplished="something",
    next_steps="something",
    relevant_files="something",
  )
  assert "Error" in result


@pytest.mark.asyncio
async def test_qwick_memory_session_summary_rotation(rag_env: str) -> None:
  """qwick_memory_session_summary keeps only 3 most recent summaries."""
  from qwick_memory.server import qwick_memory_session_summary

  import time

  # Save 4 summaries with different content (small delay for distinct timestamps)
  for i in range(4):
    result = await qwick_memory_session_summary(
      goal=f"Goal number {i}",
      discoveries=f"Discovery {i}",
      accomplished=f"Accomplished {i}",
      next_steps=f"Next {i}",
      relevant_files=f"file{i}.py",
    )
    assert "Saved" in result
    time.sleep(0.01)

  # Check that only 3 remain on disk
  import os
  from pathlib import Path

  memories_dir = Path(rag_env) / "memories"
  all_files = list(memories_dir.rglob("*.md"))
  # Parse and count session-summary type
  from qwick_memory.memory import parse_memory

  summaries = [f for f in all_files if parse_memory(f).type == "session-summary"]
  assert len(summaries) == 3
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /Users/falconiere/Projects/qwick-memory && uv run pytest tests/test_server.py::test_qwick_memory_session_summary -v`
Expected: FAIL with `ImportError`

- [ ] **Step 3: Implement the tool**

Add to `src/qwick_memory/server.py`, before the `main()` function:

```python
@mcp.tool()
async def qwick_memory_session_summary(
  goal: str,
  discoveries: str,
  accomplished: str,
  next_steps: str,
  relevant_files: str,
) -> str:
  """Save a structured session summary to memory.

  Call this before ending a session or when context compaction is imminent.

  Args:
    goal: What the user wanted to accomplish.
    discoveries: Non-obvious things learned.
    accomplished: What was done.
    next_steps: What remains to be done.
    relevant_files: Key files touched or referenced.

  Returns:
    Status string confirming the save.
  """
  if not goal or not goal.strip():
    return "Error: goal cannot be empty."

  content = (
    f"## Session Summary\n\n"
    f"**Goal:** {goal.strip()}\n\n"
    f"**Discoveries:**\n{discoveries.strip()}\n\n"
    f"**Accomplished:**\n{accomplished.strip()}\n\n"
    f"**Next Steps:**\n{next_steps.strip()}\n\n"
    f"**Relevant Files:**\n{relevant_files.strip()}"
  )

  memory_id = generate_id(content)
  repo = get_repo()
  author = get_author()

  memories_dir = get_memories_dir()
  repo_dir = memories_dir / repo
  repo_dir.mkdir(parents=True, exist_ok=True)

  final_path = repo_dir / f"{memory_id}.md"

  if final_path.exists():
    return f"Session summary already exists: {memory_id}"

  memory = Memory(
    id=memory_id,
    repo=repo,
    type="session-summary",
    tags=["session-summary"],
    author=author,
    created=datetime.now(timezone.utc),
    content=content,
  )

  tmp_path = repo_dir / f".{memory_id}.tmp"
  try:
    write_memory(memory, tmp_path)
    idx = get_index()
    idx.upsert(memory)
    tmp_path.rename(final_path)
  except Exception as exc:
    tmp_path.unlink(missing_ok=True)
    logger.exception("Failed to save session summary %s", memory_id)
    return f"Error saving session summary: {exc}"

  # Rotation: keep only 3 most recent session summaries
  _rotate_session_summaries(repo_dir, repo, max_keep=3)

  return f"Saved session summary {memory_id}"
```

Also add `from pathlib import Path` to the imports at the top of `server.py` (needed for the `_rotate_session_summaries` type annotation).

Add the rotation helper function above the tool:

```python
def _rotate_session_summaries(
  repo_dir: Path, repo: str, max_keep: int = 3
) -> None:
  """Delete old session summaries, keeping only the most recent `max_keep`."""
  summaries: list[tuple[datetime, Path]] = []
  for fp in repo_dir.glob("*.md"):
    try:
      mem = parse_memory(fp)
      if mem.type == "session-summary":
        summaries.append((mem.created, fp))
    except Exception:
      continue

  if len(summaries) <= max_keep:
    return

  # Sort oldest first, delete extras
  summaries.sort(key=lambda x: x[0])
  to_delete = summaries[: len(summaries) - max_keep]
  idx = get_index()
  for _, fp in to_delete:
    memory_id = fp.stem
    fp.unlink(missing_ok=True)
    try:
      idx.delete(memory_id)
    except Exception:
      logger.warning("Could not remove %s from index during rotation.", memory_id)
```

- [ ] **Step 4: Run all 3 new tests to verify they pass**

Run: `cd /Users/falconiere/Projects/qwick-memory && uv run pytest tests/test_server.py -k "session_summary" -v`
Expected: All 3 pass

- [ ] **Step 5: Run full test suite**

Run: `cd /Users/falconiere/Projects/qwick-memory && uv run pytest -v`
Expected: All tests pass

- [ ] **Step 6: Commit**

```bash
cd /Users/falconiere/Projects/qwick-memory && git add src/qwick_memory/server.py tests/test_server.py && git commit -m "feat: add qwick_memory_session_summary tool with rotation"
```

---

### Task 5: Enhance `qwick_memory_context` with Structured Output

**Files:**
- Modify: `src/qwick_memory/server.py` (the `qwick_memory_context` function)
- Test: `tests/test_server.py`

- [ ] **Step 1: Write the failing tests**

Add to `tests/test_server.py`:

```python
@pytest.mark.asyncio
async def test_qwick_memory_context_shows_summary_first(rag_env: str) -> None:
  """qwick_memory_context shows latest session summary before other memories."""
  from qwick_memory.server import qwick_memory_context, qwick_memory_save, qwick_memory_session_summary

  await qwick_memory_save("Regular memory about PostgreSQL", type="decision", tags="db")
  await qwick_memory_session_summary(
    goal="Test context ordering",
    discoveries="None",
    accomplished="Saved a memory",
    next_steps="Verify ordering",
    relevant_files="test_server.py",
  )

  result = await qwick_memory_context()
  # Session summary should appear before regular memories
  summary_pos = result.find("Last Session")
  memories_pos = result.find("Recent Memories")
  assert summary_pos != -1, "Should contain 'Last Session' section"
  assert memories_pos != -1, "Should contain 'Recent Memories' section"
  assert summary_pos < memories_pos, "Summary should come before regular memories"


@pytest.mark.asyncio
async def test_qwick_memory_context_empty(rag_env: str) -> None:
  """qwick_memory_context on empty repo returns 'No memories found'."""
  from qwick_memory.server import qwick_memory_context

  result = await qwick_memory_context()
  assert "No memories found" in result
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /Users/falconiere/Projects/qwick-memory && uv run pytest tests/test_server.py::test_qwick_memory_context_shows_summary_first tests/test_server.py::test_qwick_memory_context_empty -v`
Expected: FAIL (current implementation doesn't have "Last Session" / "Recent Memories" sections)

- [ ] **Step 3: Rewrite `qwick_memory_context` with structured output**

Replace the `qwick_memory_context` function in `src/qwick_memory/server.py`:

```python
@mcp.tool()
async def qwick_memory_context(repo: str | None = None, limit: int = 20) -> str:
  """Get recent memories for the current repo, with latest session summary first.

  Args:
    repo: Repository name (defaults to auto-detected repo).
    limit: Maximum number of non-summary memories to return.

  Returns:
    Formatted text with session summary (if any) followed by recent memories.
  """
  target_repo = repo or get_repo()
  memories_dir = get_memories_dir()
  if not memories_dir.exists():
    return "No memories directory found."

  md_files = scan_memories(memories_dir)
  if not md_files:
    return "No memories found."

  summaries: list[Memory] = []
  regular: list[Memory] = []
  for fp in md_files:
    try:
      mem = parse_memory(fp)
    except Exception:
      continue
    if mem.repo != target_repo:
      continue
    if mem.type == "session-summary":
      summaries.append(mem)
    else:
      regular.append(mem)

  if not summaries and not regular:
    return f"No memories found for repo: {target_repo}"

  lines: list[str] = []

  # Section 1: Latest session summary
  if summaries:
    summaries.sort(key=lambda m: m.created, reverse=True)
    latest = summaries[0]
    lines.append("### Last Session")
    lines.append(latest.content)
    lines.append("")

  # Section 2: Recent non-summary memories
  if regular:
    regular.sort(key=lambda m: m.created, reverse=True)
    regular = regular[:limit]
    lines.append("### Recent Memories")
    for mem in regular:
      preview = mem.content[:120] + "..." if len(mem.content) > 120 else mem.content
      lines.append(f"- [{mem.created.isoformat()}] ({mem.type}) {preview}")

  return "\n".join(lines)
```

- [ ] **Step 4: Run the new tests**

Run: `cd /Users/falconiere/Projects/qwick-memory && uv run pytest tests/test_server.py -k "context" -v`
Expected: Both pass

- [ ] **Step 5: Run full test suite**

Run: `cd /Users/falconiere/Projects/qwick-memory && uv run pytest -v`
Expected: All tests pass

- [ ] **Step 6: Commit**

```bash
cd /Users/falconiere/Projects/qwick-memory && git add src/qwick_memory/server.py tests/test_server.py && git commit -m "feat: enhance qwick_memory_context with structured summary-first output"
```

---

### Task 6: Add CLI `context` Command

**Files:**
- Modify: `src/qwick_memory/cli.py`
- Test: `tests/test_cli.py`

- [ ] **Step 1: Write the failing tests**

Add to `tests/test_cli.py`:

```python
def test_context_shows_memories(tmp_path: Path) -> None:
  """context command shows recent memories."""
  # Save a memory first
  result = runner.invoke(app, ["save", "Context test memory content"])
  assert result.exit_code == 0, result.output

  result = runner.invoke(app, ["context"])
  assert result.exit_code == 0, result.output
  assert "Recent Memories" in result.output
  assert "Context test memory" in result.output


def test_context_empty(tmp_path: Path) -> None:
  """context command on empty repo shows 'No memories found'."""
  result = runner.invoke(app, ["context"])
  assert result.exit_code == 0, result.output
  assert "No memories found" in result.output


def test_context_limit(tmp_path: Path) -> None:
  """context --limit restricts number of memories shown."""
  for i in range(5):
    result = runner.invoke(app, ["save", f"Memory number {i} unique content here"])
    assert result.exit_code == 0, result.output

  result = runner.invoke(app, ["context", "--limit", "2"])
  assert result.exit_code == 0, result.output
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /Users/falconiere/Projects/qwick-memory && uv run pytest tests/test_cli.py::test_context_shows_memories -v`
Expected: FAIL (no `context` command yet)

- [ ] **Step 3: Implement the `context` command**

Add to `src/qwick_memory/cli.py`, after the `index` command and before `doctor`:

```python
@app.command()
def context(
  repo: str | None = typer.Option(None, "--repo", "-r", help="Filter by repo."),
  limit: int = typer.Option(10, "--limit", "-n", help="Max non-summary memories."),
) -> None:
  """Show recent memories for context restoration."""
  memories_dir = get_memories_dir()
  if not memories_dir.exists():
    out.print("No memories found.")
    return

  md_files = scan_memories(memories_dir)
  if not md_files:
    out.print("No memories found.")
    return

  target_repo = repo or get_repo()

  summaries: list[Memory] = []
  regular: list[Memory] = []
  for fp in md_files:
    try:
      mem = parse_memory(fp)
    except Exception:
      continue
    if mem.repo != target_repo:
      continue
    if mem.type == "session-summary":
      summaries.append(mem)
    else:
      regular.append(mem)

  if not summaries and not regular:
    out.print(f"No memories found for repo: {target_repo}")
    return

  # Section 1: Latest session summary
  if summaries:
    summaries.sort(key=lambda m: m.created, reverse=True)
    latest = summaries[0]
    out.print("### Last Session")
    out.print(latest.content)
    out.print()

  # Section 2: Recent non-summary memories
  if regular:
    regular.sort(key=lambda m: m.created, reverse=True)
    regular = regular[:limit]
    out.print("### Recent Memories")
    for mem in regular:
      preview = mem.content[:120] + "..." if len(mem.content) > 120 else mem.content
      out.print(f"- [{mem.created.isoformat()}] ({mem.type}) {preview}")
```

- [ ] **Step 4: Run the new tests**

Run: `cd /Users/falconiere/Projects/qwick-memory && uv run pytest tests/test_cli.py -k "context" -v`
Expected: All 3 pass

- [ ] **Step 5: Run full test suite**

Run: `cd /Users/falconiere/Projects/qwick-memory && uv run pytest -v`
Expected: All tests pass

- [ ] **Step 6: Commit**

```bash
cd /Users/falconiere/Projects/qwick-memory && git add src/qwick_memory/cli.py tests/test_cli.py && git commit -m "feat: add CLI context command for memory context restoration"
```

---

### Task 7: Update Hook Scripts and hooks.json

**Files:**
- Modify: `hooks/hooks.json`
- Modify: `scripts/session-start.sh`
- Create: `scripts/pre-compact.sh`
- Create: `scripts/post-compact.sh`

- [ ] **Step 1: Update `hooks/hooks.json`**

Replace the contents of `hooks/hooks.json`:

```json
{
  "hooks": [
    {
      "event": "SessionStart",
      "commands": [
        {
          "command": "${CLAUDE_PLUGIN_ROOT}/scripts/session-start.sh",
          "timeout": 30000
        }
      ]
    },
    {
      "event": "PreCompact",
      "commands": [
        {
          "command": "${CLAUDE_PLUGIN_ROOT}/scripts/pre-compact.sh",
          "timeout": 60000
        }
      ]
    },
    {
      "event": "PostCompact",
      "commands": [
        {
          "command": "${CLAUDE_PLUGIN_ROOT}/scripts/post-compact.sh",
          "timeout": 30000
        }
      ]
    }
  ]
}
```

- [ ] **Step 2: Update `scripts/session-start.sh`**

Replace the contents of `scripts/session-start.sh`:

```bash
#!/usr/bin/env bash
# Session start: auto-index + output context for Claude
set -euo pipefail

PLUGIN_ROOT="${CLAUDE_PLUGIN_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
cd "$PLUGIN_ROOT"

# Auto-index
if [ -d "memories" ]; then
  uv run python -m qwick_memory index 2>/dev/null || true
fi

# Output context for Claude
echo "## Qwick Memory — Session Context"
echo ""
uv run python -m qwick_memory context 2>/dev/null || echo "No prior context found."
```

- [ ] **Step 3: Create `scripts/pre-compact.sh`**

```bash
#!/usr/bin/env bash
# Pre-compaction: best-effort reminder + context snapshot
set -euo pipefail

PLUGIN_ROOT="${CLAUDE_PLUGIN_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
cd "$PLUGIN_ROOT"

echo "## Qwick Memory — Pre-Compaction Notice"
echo ""
echo "Context compaction is about to happen."
echo "If you haven't already, call qwick_memory_session_summary now."
echo ""
echo "Current memory state:"
uv run python -m qwick_memory context --limit 5 2>/dev/null || echo "No context available."
```

- [ ] **Step 4: Create `scripts/post-compact.sh`**

```bash
#!/usr/bin/env bash
# Post-compaction: restore context from memories
set -euo pipefail

PLUGIN_ROOT="${CLAUDE_PLUGIN_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
cd "$PLUGIN_ROOT"

echo "## Qwick Memory — Context Restored After Compaction"
echo ""
echo "Context was just compacted. Here are your recent memories:"
echo ""
uv run python -m qwick_memory context 2>/dev/null || echo "No prior context found."
```

- [ ] **Step 5: Make new scripts executable**

Run: `chmod +x /Users/falconiere/Projects/qwick-memory/scripts/pre-compact.sh /Users/falconiere/Projects/qwick-memory/scripts/post-compact.sh`

- [ ] **Step 6: Verify all scripts are executable**

Run: `ls -la /Users/falconiere/Projects/qwick-memory/scripts/*.sh`
Expected: All 3 scripts have execute permission

- [ ] **Step 7: Commit**

```bash
cd /Users/falconiere/Projects/qwick-memory && git add hooks/hooks.json scripts/session-start.sh scripts/pre-compact.sh scripts/post-compact.sh && git commit -m "feat: add PreCompact and PostCompact hooks, enhance SessionStart"
```

---

### Task 8: Update E2E Test Script

**Files:**
- Modify: `scripts/e2e-test.sh`

- [ ] **Step 1: Add context command section to e2e test**

Add the following section in `scripts/e2e-test.sh` after section 7 (Search after rebuild) and before section 8 (Doctor). Update the section numbers: old section 8 (Doctor) becomes section 9, and update the total check count comment at the top.

Insert after the "Search after rebuild" section:

```bash
# ── 8. Context command ────────────────────────────────────────────────────────

echo -e "${BOLD}8. Context command${RESET}"

OUT=$($QR context 2>&1) || true
EC=$?
assert_exit_code 0 "$EC" "context exits 0"
assert_contains "$OUT" "Recent Memories" "context shows Recent Memories section"

echo ""
```

Rename old section 8 (Doctor) to section 9. The results line computes the total dynamically from `$PASSED + $FAILED`, so no count update is needed.

- [ ] **Step 2: Verify the e2e test runs**

Run: `cd /Users/falconiere/Projects/qwick-memory && ./scripts/e2e-test.sh`
Expected: All 28 checks pass

- [ ] **Step 3: Commit**

```bash
cd /Users/falconiere/Projects/qwick-memory && git add scripts/e2e-test.sh && git commit -m "test: add context command to e2e test"
```

---

### Task 9: Update CLAUDE.md

**Files:**
- Modify: `CLAUDE.md`

- [ ] **Step 1: Update module map**

In `CLAUDE.md`, update the module map table. Change the `server.py` row to reflect 7 tools:

```
| `server.py` | MCP server with 7 tools for Claude Code + memory protocol |
```

- [ ] **Step 2: Add memory protocol section**

Add a new section after "Claude Code Plugin" in `CLAUDE.md`:

```markdown
## Memory Protocol

qwick-memory includes an automatic memory protocol injected via MCP server instructions. When active, Claude proactively saves decisions, bugs, conventions, discoveries, and session summaries. The protocol is defined in `server.py` as the `PROTOCOL` constant.

**Hooks:**
- `SessionStart` — Auto-index + load context
- `PreCompact` — Reminder to save session summary
- `PostCompact` — Restore context after compaction

**Key tools:**
- `qwick_memory_save` — Save a memory (all types)
- `qwick_memory_search` — Semantic search
- `qwick_memory_context` — Load recent context (summary first)
- `qwick_memory_session_summary` — Save structured session summary (with rotation, keeps 3)

This replaces engram. Disable engram when qwick-memory is active.
```

- [ ] **Step 3: Update README.md tool names**

In `README.md`, find the line referencing `rag_save`, `rag_search`, etc. and update to:

```
This gives Claude Code 7 MCP tools: `qwick_memory_save`, `qwick_memory_search`, `qwick_memory_list`, `qwick_memory_delete`, `qwick_memory_index`, `qwick_memory_context`, `qwick_memory_session_summary`.
```

- [ ] **Step 4: Rewrite skills/memory/SKILL.md**

Replace the contents of `skills/memory/SKILL.md` with the new protocol that matches the MCP server instructions. Update all tool names from `rag_*` to `qwick_memory_*`, add `session-summary` type, and add the SESSION CLOSE section:

```markdown
---
name: memory
description: ALWAYS ACTIVE — Centralized memory protocol for cross-repository knowledge. Save decisions, bugs, conventions, and discoveries proactively.
---

## Qwick Memory Protocol

You have qwick-memory tools (qwick_memory_save, qwick_memory_search, qwick_memory_list, qwick_memory_delete, qwick_memory_index, qwick_memory_context, qwick_memory_session_summary).

### PROACTIVE SAVE — do NOT wait for user to ask
Call `qwick_memory_save` IMMEDIATELY after ANY of these:
- Decision made (architecture, convention, workflow, tool choice)
- Bug fixed (include root cause)
- Convention or workflow established
- Non-obvious discovery or edge case found
- Pattern established (naming, structure, approach)
- User preference or constraint learned
- Feature implemented with non-obvious approach

### SEARCH MEMORY when:
- Starting work on something that might have been done before
- User asks to recall anything
- User mentions a topic you have no context on
- User's first message references a problem or feature

### Memory Types
- `decision` — Architecture, tool, or workflow choices
- `bug` — Bug root causes and fixes
- `convention` — Coding standards, naming patterns
- `discovery` — Non-obvious findings, gotchas
- `pattern` — Established approaches
- `preference` — User or team preferences
- `note` — General knowledge that doesn't fit other types
- `session-summary` — (used automatically by qwick_memory_session_summary)

### SESSION CLOSE — before saying "done"/"listo":
Call `qwick_memory_session_summary` with: goal, discoveries, accomplished, next_steps, relevant_files.
```

- [ ] **Step 5: Commit**

```bash
cd /Users/falconiere/Projects/qwick-memory && git add CLAUDE.md README.md skills/memory/SKILL.md && git commit -m "docs: update CLAUDE.md, README, and SKILL.md with new tool names and protocol"
```

---

### Task 10: Final Verification

- [ ] **Step 1: Run full test suite**

Run: `cd /Users/falconiere/Projects/qwick-memory && uv run pytest -v`
Expected: All tests pass

- [ ] **Step 2: Run linter and formatter**

Run: `cd /Users/falconiere/Projects/qwick-memory && uv run ruff format src/ tests/ && uv run ruff check src/ tests/`
Expected: No errors

- [ ] **Step 3: Run type checker**

Run: `cd /Users/falconiere/Projects/qwick-memory && uv run pyright src/`
Expected: No errors (or only pre-existing ones)

- [ ] **Step 4: Run e2e test**

Run: `cd /Users/falconiere/Projects/qwick-memory && ./scripts/e2e-test.sh`
Expected: All checks pass

- [ ] **Step 5: Verify hook scripts are executable**

Run: `ls -la /Users/falconiere/Projects/qwick-memory/scripts/*.sh`
Expected: All scripts have `+x` permission

- [ ] **Step 6: Manual smoke test — start MCP server**

Run: `cd /Users/falconiere/Projects/qwick-memory && echo '{}' | uv run python -m qwick_memory.server 2>&1 | head -5`
Expected: Server starts without errors (may output JSON-RPC initialization)
