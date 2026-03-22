# Aggressive Plugin Discoverability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the qwick-memory plugin more aggressive (reliable search-before-answer and save-after-work) and more discoverable (for both Claude and users).

**Architecture:** Multi-layer behavioral reinforcement — tool descriptions carry trigger conditions (highest priority for Claude's tool-calling), PROTOCOL uses a decision tree structure, SessionStart hook adds a compact reminder footer, and tool responses include next-action hints. All surfaces are concise and non-redundant.

**Tech Stack:** Python (FastMCP server), Bash (hook scripts), JSON (plugin manifests), Markdown (skill descriptor)

**Spec:** `docs/superpowers/specs/2026-03-22-aggressive-plugin-design.md`

---

### Task 1: Write tests for new response hints

**Files:**
- Modify: `tests/test_server.py`

These tests verify the new response strings contain the expected behavioral hints. They run alongside existing tests (which use `in` checks and will continue to pass).

- [ ] **Step 1: Write test for save response hint**

Add to `tests/test_server.py`:

```python
@pytest.mark.asyncio
async def test_save_response_includes_vector_hint(rag_env: str) -> None:
  """qwick_memory_save response mentions vector search indexing."""
  from qwick_memory.server import qwick_memory_save

  result = await qwick_memory_save("Test memory for hint check", type="decision")
  assert "Embedded and indexed for vector search" in result
  assert "(decision)" in result
```

- [ ] **Step 2: Write test for save duplicate response hint**

```python
@pytest.mark.asyncio
async def test_save_duplicate_response_hint(rag_env: str) -> None:
  """qwick_memory_save duplicate response includes 'no action needed' hint."""
  from qwick_memory.server import qwick_memory_save

  await qwick_memory_save("Duplicate hint test content")
  result = await qwick_memory_save("Duplicate hint test content")
  assert "already exists" in result
  assert "No action needed" in result
```

- [ ] **Step 3: Write test for search results response hint**

```python
@pytest.mark.asyncio
async def test_search_results_include_similarity_hint(rag_env: str) -> None:
  """qwick_memory_search results include semantic similarity hint."""
  from qwick_memory.server import qwick_memory_save, qwick_memory_search

  await qwick_memory_save("Redis is used for caching")
  result = await qwick_memory_search("Redis caching")
  assert "Results ranked by semantic similarity" in result
```

- [ ] **Step 4: Write test for search no-results response hint**

```python
@pytest.mark.asyncio
async def test_search_no_results_includes_save_hint(rag_env: str) -> None:
  """qwick_memory_search with no results hints to save later."""
  from qwick_memory.server import qwick_memory_search

  result = await qwick_memory_search("completely nonexistent topic xyz123")
  assert "save it with qwick_memory_save" in result
```

- [ ] **Step 5: Write test for index response hint**

```python
@pytest.mark.asyncio
async def test_index_response_includes_vector_hint(rag_env: str) -> None:
  """qwick_memory_index response mentions vector search."""
  from qwick_memory.server import qwick_memory_index

  result = await qwick_memory_index()
  assert "searchable by semantic similarity" in result
```

- [ ] **Step 6: Write test for delete response hint**

```python
@pytest.mark.asyncio
async def test_delete_response_confirms_both_layers(rag_env: str) -> None:
  """qwick_memory_delete response confirms disk and vector index removal."""
  from qwick_memory.server import qwick_memory_delete, qwick_memory_save

  result = await qwick_memory_save("Memory to delete for hint test")
  # Format: "Saved memory {id} ..." — ID is the third word
  memory_id = result.split()[2]

  result = await qwick_memory_delete(memory_id)
  assert "Removed from disk and vector index" in result
```

- [ ] **Step 7: Write test for session summary response hint**

```python
@pytest.mark.asyncio
async def test_session_summary_response_includes_vector_hint(rag_env: str) -> None:
  """qwick_memory_session_summary response mentions vector search."""
  from qwick_memory.server import qwick_memory_session_summary

  result = await qwick_memory_session_summary(
    goal="Test hint in session summary",
    discoveries="None",
    accomplished="Testing",
    next_steps="Verify",
    relevant_files="test_server.py",
  )
  assert "Embedded and indexed for vector search" in result
```

- [ ] **Step 8: Run tests to verify they all FAIL**

Run: `cd /Users/falconiere/Projects/qwick-rag && uv run pytest tests/test_server.py -v`

Expected: The 7 new tests FAIL (response strings don't contain hints yet). All existing tests PASS.

- [ ] **Step 9: Commit failing tests**

```bash
git add tests/test_server.py
git commit -m "test: add failing tests for response hint strings"
```

---

### Task 2: Rewrite PROTOCOL constant

**Files:**
- Modify: `src/qwick_memory/server.py:27-74`

Replace the flat bullet-list PROTOCOL with the decision tree structure.

- [ ] **Step 1: Replace PROTOCOL constant**

In `src/qwick_memory/server.py`, replace lines 27-74 (the entire `PROTOCOL = """\` block) with:

```python
PROTOCOL = """\
## Qwick Memory — ACTIVE PROTOCOL

You have qwick-memory tools. This protocol is MANDATORY.

### DECISION TREE — follow on EVERY user message:

**Step 1: Should I SEARCH first?**
User message contains ANY of these → call `qwick_memory_search` BEFORE responding:
- Questions: "what", "why", "how", "remember", "recall", "what did we"
- References: PR, issue, bug, feature, decision, architecture
- Implicit: user assumes you have context you don't have
- First message: always search with project/topic keywords
→ If unsure, SEARCH. Cost of an unnecessary search is near zero.
  Cost of missing context is a wrong answer.
→ For "where were we?" or status requests, prefer `qwick_memory_context`.

**Step 2: Should I SAVE after?**
You just did ANY of these → call `qwick_memory_save` IMMEDIATELY:
- Made a decision (architecture, convention, workflow, tool choice)
- Fixed a bug (save root cause + fix)
- Discovered something non-obvious (gotcha, edge case, quirk)
- Established a pattern or convention
- Learned a user preference or constraint
- Created/updated an artifact (PR, doc, config, Jira, Notion)
→ If unsure, SAVE. A redundant memory is better than a lost insight.

**Step 3: Is this session ending?**
User signals completion → call `qwick_memory_session_summary`:
- "done", "listo", "thanks", "that's it", "bye"
- Context compaction imminent
- Major milestone completed
"""
```

- [ ] **Step 2: Run existing tests to confirm no breakage**

Run: `cd /Users/falconiere/Projects/qwick-rag && uv run pytest tests/test_server.py -v -k "not hint"`

Expected: All existing tests PASS (PROTOCOL change doesn't affect tool behavior).

- [ ] **Step 3: Commit**

```bash
git add src/qwick_memory/server.py
git commit -m "refactor: replace PROTOCOL flat list with decision tree"
```

---

### Task 3: Rewrite tool descriptions

**Files:**
- Modify: `src/qwick_memory/server.py` — docstrings for `qwick_memory_save`, `qwick_memory_search`, `qwick_memory_context`, `qwick_memory_session_summary`

Each tool description now leads with behavioral triggers (WHEN to call) before the functional description.

- [ ] **Step 1: Rewrite `qwick_memory_search` docstring**

Replace the docstring of `qwick_memory_search` (lines 146-157 of the original, adjust after Task 2) with:

```python
  """Search memories by semantic similarity.

  CALL THIS TOOL FIRST when:
  - User asks about prior work, PRs, decisions, bugs, or project history
  - User asks "what do you know about...", "remember...", "what did we do..."
  - Starting work that might overlap with previous sessions
  - User's first message references a feature, problem, or project area
  - You lack context on a topic the user is discussing

  -> Always search BEFORE answering from general knowledge.
  Memory has project-specific context you don't.

  Default: search all repos. Use the repo filter only when the user
  explicitly scopes to one project.

  Args:
    query: Search query text.
    repo: Filter by repository name.
    type: Filter by memory type.
    tag: Filter by tag.
    limit: Maximum number of results.

  Returns:
    Formatted text with search results ranked by semantic similarity.
  """
```

- [ ] **Step 2: Rewrite `qwick_memory_save` docstring**

Replace the docstring of `qwick_memory_save` with:

```python
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

  Returns:
    Status string confirming the save with indexing details.
  """
```

- [ ] **Step 3: Rewrite `qwick_memory_context` docstring**

Replace the docstring of `qwick_memory_context` with:

```python
  """Get recent memories for context. Includes latest session summary + recent memories.

  CALL THIS when:
  - Starting a new session (if SessionStart hook didn't fire)
  - Resuming work after a pause
  - User asks for a status update or "where were we?"

  Args:
    repo: Repository name (defaults to auto-detected repo).
    limit: Maximum number of non-summary memories to return.

  Returns:
    Formatted text with session summary (if any) followed by recent memories.
  """
```

- [ ] **Step 4: Rewrite `qwick_memory_session_summary` docstring**

Replace the docstring of `qwick_memory_session_summary` with:

```python
  """Save a structured session summary. MUST be called before ending a session.

  CALL THIS when:
  - User says "done", "listo", "that's it", "thanks", or signals session end
  - Before context compaction
  - After completing a significant milestone

  Args:
    goal: What the user wanted to accomplish.
    discoveries: Non-obvious things learned.
    accomplished: What was done.
    next_steps: What remains to be done.
    relevant_files: Key files touched or referenced.

  Returns:
    Status string confirming the save with indexing details.
  """
```

- [ ] **Step 5: Run existing tests**

Run: `cd /Users/falconiere/Projects/qwick-rag && uv run pytest tests/test_server.py -v -k "not hint"`

Expected: All existing tests PASS (docstring changes don't affect behavior).

- [ ] **Step 6: Commit**

```bash
git add src/qwick_memory/server.py
git commit -m "feat: add behavioral triggers to tool descriptions"
```

---

### Task 4: Update response strings with hints

**Files:**
- Modify: `src/qwick_memory/server.py` — return strings in `qwick_memory_save`, `qwick_memory_search`, `qwick_memory_delete`, `qwick_memory_index`, `qwick_memory_session_summary`

- [ ] **Step 1: Update `qwick_memory_save` success response**

In `qwick_memory_save`, change the success return (currently `return f"Saved memory {memory_id}"`) to:

```python
  return (
    f"Saved memory {memory_id} ({type}). Embedded and indexed for vector search.\n"
    f"-> This memory is now searchable by semantic similarity across all future sessions."
  )
```

- [ ] **Step 2: Update `qwick_memory_save` duplicate response**

In `qwick_memory_save`, change the duplicate return (currently `return f"Memory already exists: {memory_id}"`) to:

```python
  return (
    f"Memory already exists: {memory_id}. Content was previously saved.\n"
    f"-> No action needed. The memory is already indexed and searchable."
  )
```

- [ ] **Step 3: Update `qwick_memory_search` results response**

In `qwick_memory_search`, change the results return (currently `return "\n".join(lines)`) to:

```python
  result = "\n".join(lines)
  return f"{result}\n-> Results ranked by semantic similarity. Use these memories to inform your response."
```

- [ ] **Step 4: Update `qwick_memory_search` no-results response**

In `qwick_memory_search`, change the no-results return (currently `return "No results found."`) to:

```python
  return (
    "No semantically similar memories found.\n"
    "-> If you learn something new about this topic, save it with qwick_memory_save "
    "so future searches can find it."
  )
```

- [ ] **Step 5: Update `qwick_memory_delete` response**

In `qwick_memory_delete`, change the success return (currently `return f"Deleted memory {memory_id}"`) to:

```python
  return f"Deleted memory {memory_id}. Removed from disk and vector index."
```

- [ ] **Step 6: Update `qwick_memory_index` response**

In `qwick_memory_index`, change the return to:

```python
  return (
    f"Indexed: {stats['new']} new, {stats['updated']} updated, "
    f"{stats['deleted']} deleted. Total: {idx.count()} vectors.\n"
    f"-> Vector index rebuilt. All memories are now searchable by semantic similarity."
  )
```

- [ ] **Step 7: Update `qwick_memory_session_summary` success response**

In `qwick_memory_session_summary`, change the success return (currently `return f"Saved session summary {memory_id}"`) to:

```python
  return (
    f"Saved session summary {memory_id}. Embedded and indexed for vector search.\n"
    f"-> Session context preserved for next time."
  )
```

- [ ] **Step 8: Run ALL tests (new + existing)**

Run: `cd /Users/falconiere/Projects/qwick-rag && uv run pytest tests/test_server.py -v`

Expected: ALL tests PASS — both existing (using `in` checks) and new hint tests.

- [ ] **Step 9: Commit**

```bash
git add src/qwick_memory/server.py
git commit -m "feat: add behavioral hints to tool response strings"
```

---

### Task 5: Update SessionStart hook footer

**Files:**
- Modify: `scripts/session-start.sh`

Add a 3-line decision guide footer as static `echo` statements after the context output.

- [ ] **Step 1: Add footer to session-start.sh**

Replace the current content of `scripts/session-start.sh` with:

```bash
#!/usr/bin/env bash
# Session start: auto-index + output context + protocol reminder for Claude
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
UV="uv run --directory $PROJECT_ROOT"

# Auto-index
if [ -d "$PROJECT_ROOT/memories" ]; then
  $UV python -m qwick_memory index 2>/dev/null || true
fi

# Output context for Claude
echo "## Qwick Memory — Session Context"
echo ""
$UV python -m qwick_memory context 2>/dev/null || echo "No prior context found."

# Decision guide footer (high-attention position at end of session start)
echo ""
echo "---"
echo "Memory Protocol Active:"
echo "-> SEARCH before answering questions about prior work, PRs, decisions, or history"
echo "-> SAVE after decisions, bug fixes, discoveries, conventions, preferences"
echo "-> SUMMARIZE before ending session"
```

- [ ] **Step 2: Verify script runs without errors**

Run: `cd /Users/falconiere/Projects/qwick-rag && QWICK_MEMORY_DIR=/tmp/qwick-test-$$ QWICK_MEMORY_REPO=test QWICK_MEMORY_AUTHOR=test bash scripts/session-start.sh`

Expected: Outputs context header + "No prior context found." + decision guide footer. No errors.

- [ ] **Step 3: Commit**

```bash
git add scripts/session-start.sh
git commit -m "feat: add protocol reminder footer to SessionStart hook"
```

---

### Task 6: Trim SKILL.md to minimal pointer

**Files:**
- Modify: `skills/memory/SKILL.md`

The current SKILL.md duplicates the protocol as a flat bullet list. Trim it to a minimal pointer that defers to tool descriptions and PROTOCOL.

- [ ] **Step 1: Replace SKILL.md content**

Replace the entire content of `skills/memory/SKILL.md` with:

```markdown
---
name: memory
description: ALWAYS ACTIVE — Centralized memory protocol for cross-repository knowledge. Save decisions, bugs, conventions, and discoveries proactively.
---

## Qwick Memory Protocol

You have qwick-memory tools. The protocol is MANDATORY — see each tool's description for when to call it.

Key behavior:
- **SEARCH first** when the user asks about prior work, decisions, bugs, or history
- **SAVE after** decisions, bug fixes, discoveries, conventions, preferences
- **SUMMARIZE** before ending a session

Tool descriptions contain full trigger conditions. Follow them.
```

- [ ] **Step 2: Commit**

```bash
git add skills/memory/SKILL.md
git commit -m "refactor: trim SKILL.md to minimal pointer, defer to tool descriptions"
```

---

### Task 7: Update marketplace and plugin metadata

**Files:**
- Modify: `.claude-plugin/marketplace.json`
- Modify: `.claude-plugin/plugin.json`

- [ ] **Step 1: Update marketplace.json description**

In `.claude-plugin/marketplace.json`, change the `description` field in the plugin entry to:

```json
"description": "Persistent developer memory across repos — automatically saves decisions, bugs, conventions, and discoveries. Semantic vector search recalls prior work so Claude never forgets what you've built."
```

- [ ] **Step 2: Update plugin.json description**

In `.claude-plugin/plugin.json`, change the `description` field to:

```json
"description": "Persistent developer memory with semantic vector search — Claude automatically saves and recalls decisions, bugs, conventions, and discoveries across repositories."
```

- [ ] **Step 3: Commit**

```bash
git add .claude-plugin/marketplace.json .claude-plugin/plugin.json
git commit -m "feat: update plugin descriptions for better discoverability"
```

---

### Task 8: Full verification

**Files:** None (verification only)

- [ ] **Step 1: Run full unit test suite**

Run: `cd /Users/falconiere/Projects/qwick-rag && uv run pytest -v`

Expected: All tests pass (existing + new hint tests).

- [ ] **Step 2: Run e2e test**

Run: `cd /Users/falconiere/Projects/qwick-rag && ./scripts/e2e-test.sh`

Expected: All 28 checks pass. The e2e test uses the CLI (`qwick-memory`), which is separate from the MCP server. CLI response strings are in `cli.py` and are not modified by this plan.

- [ ] **Step 3: Run linter and formatter**

Run: `cd /Users/falconiere/Projects/qwick-rag && uv run ruff format src/ tests/ && uv run ruff check src/ tests/`

Expected: No issues (2-space indent enforced by ruff config).

- [ ] **Step 4: Run type checker**

Run: `cd /Users/falconiere/Projects/qwick-rag && uv run pyright src/`

Expected: No type errors.

- [ ] **Step 5: Verify token budget**

Count the approximate tokens in:
- PROTOCOL constant (should be ~200 tokens, down from ~280)
- Tool descriptions total (should be ~350 tokens, up from ~120)
- Response hints total (should be ~80 tokens, up from ~20)
- SessionStart footer (~30 tokens, new)

Total delta should be ~+240 tokens.

- [ ] **Step 6: Final commit (if linter/formatter made changes)**

```bash
git add src/ tests/
git commit -m "style: format after aggressive plugin changes"
```
