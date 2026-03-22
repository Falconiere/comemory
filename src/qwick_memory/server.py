"""MCP server for qwick-memory: exposes RAG tools for Claude Code integration."""

from __future__ import annotations

import logging
import sys
from datetime import datetime, timezone
from pathlib import Path  # noqa: TC003 — used at runtime in _rotate_session_summaries

from mcp.server.fastmcp import FastMCP

from qwick_memory.config import get_author, get_index, get_memories_dir, get_rag_dir, get_repo
from qwick_memory.git_utils import git_sync
from qwick_memory.memory import (
  MEMORY_TYPES,
  Memory,
  generate_id,
  parse_memory,
  scan_memories,
  write_memory,
)
from qwick_memory.search import search_memories

logging.basicConfig(stream=sys.stderr, level=logging.INFO)
logger = logging.getLogger(__name__)

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

mcp = FastMCP("qwick-memory", instructions=PROTOCOL)


@mcp.tool()
async def qwick_memory_save(content: str, type: str = "note", tags: str = "") -> str:
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
  if not content or not content.strip():
    return "Error: content cannot be empty."

  content = content.strip()

  if type not in MEMORY_TYPES:
    return f"Error: Invalid type '{type}'. Must be one of: {', '.join(MEMORY_TYPES)}"

  memory_id = generate_id(content)
  tag_list = [t.strip() for t in tags.split(",") if t.strip()]
  repo = get_repo()
  author = get_author()

  memories_dir = get_memories_dir()
  repo_dir = memories_dir / repo
  repo_dir.mkdir(parents=True, exist_ok=True)

  final_path = repo_dir / f"{memory_id}.md"

  if final_path.exists():
    return (
      f"Memory already exists: {memory_id}. Content was previously saved.\n"
      f"-> No action needed. The memory is already indexed and searchable."
    )

  memory = Memory(
    id=memory_id,
    repo=repo,
    type=type,
    tags=tag_list,
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
    logger.exception("Failed to save memory %s", memory_id)
    return f"Error saving memory: {exc}"

  git_sync(get_rag_dir(), f"save: {memory_id} ({type})")
  return (
    f"Saved memory {memory_id} ({type}). Embedded and indexed for vector search.\n"
    f"-> This memory is now searchable by semantic similarity across all future sessions."
  )


@mcp.tool()
async def qwick_memory_search(
  query: str,
  repo: str | None = None,
  type: str | None = None,
  tag: str | None = None,
  limit: int = 10,
) -> str:
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
  idx = get_index()
  results = search_memories(idx, query, repo=repo, type_filter=type, tag=tag, limit=limit)

  if not results:
    return (
      "No semantically similar memories found.\n"
      "-> If you learn something new about this topic, save it with qwick_memory_save "
      "so future searches can find it."
    )

  lines = []
  for r in results:
    preview = r.content[:80] + "..." if len(r.content) > 80 else r.content
    lines.append(f"[{r.score:.3f}] {r.repo} ({r.type}) {preview} — {r.id}")
  result = "\n".join(lines)
  return (
    f"{result}\n"
    f"-> Results ranked by semantic similarity. Use these memories to inform your response."
  )


@mcp.tool()
async def qwick_memory_list(repo: str | None = None, type: str | None = None) -> str:
  """List memories from disk.

  Args:
    repo: Filter by repository name.
    type: Filter by memory type.

  Returns:
    Formatted text list of memories.
  """
  memories_dir = get_memories_dir()
  if not memories_dir.exists():
    return "No memories directory found."

  md_files = scan_memories(memories_dir)
  if not md_files:
    return "No memories found."

  lines = []
  for fp in md_files:
    try:
      mem = parse_memory(fp)
    except Exception:
      continue

    if repo and mem.repo != repo:
      continue
    if type and mem.type != type:
      continue

    preview = mem.content[:60] + "..." if len(mem.content) > 60 else mem.content
    tag_str = ", ".join(mem.tags) if mem.tags else ""
    lines.append(f"{mem.id} | {mem.repo} | {mem.type} | [{tag_str}] | {preview}")

  if not lines:
    return "No memories match the filters."
  return f"{len(lines)} memories:\n" + "\n".join(lines)


@mcp.tool()
async def qwick_memory_delete(memory_id: str) -> str:
  """Delete a memory by ID.

  Args:
    memory_id: The ID of the memory to delete.

  Returns:
    Status string confirming the deletion.
  """
  memories_dir = get_memories_dir()
  if not memories_dir.exists():
    return "Error: Memories directory not found."

  matches = list(memories_dir.rglob(f"{memory_id}.md"))
  if not matches:
    return f"Error: Memory file not found: {memory_id}"

  filepath = matches[0]
  filepath.unlink()

  try:
    idx = get_index()
    idx.delete(memory_id)
  except Exception:
    logger.warning("Could not remove %s from index.", memory_id)

  git_sync(get_rag_dir(), f"delete: {memory_id}")
  return (
    f"Deleted memory {memory_id}. Removed from disk and vector index.\n"
    f"-> Memory is no longer searchable."
  )


@mcp.tool()
async def qwick_memory_index(force: bool = False) -> str:
  """Build or rebuild the vector index.

  Args:
    force: Force full rebuild of the index.

  Returns:
    Status string with indexing statistics.
  """
  memories_dir = get_memories_dir()
  memories_dir.mkdir(parents=True, exist_ok=True)

  idx = get_index()
  stats = idx.build(memories_dir, force=force)

  return (
    f"Indexed: {stats['new']} new, {stats['updated']} updated, "
    f"{stats['deleted']} deleted. Total: {idx.count()} vectors.\n"
    f"-> Vector index rebuilt. All memories are now searchable by semantic similarity."
  )


@mcp.tool()
async def qwick_memory_context(repo: str | None = None, limit: int = 20) -> str:
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


@mcp.tool()
async def qwick_memory_session_summary(
  goal: str,
  discoveries: str,
  accomplished: str,
  next_steps: str,
  relevant_files: str,
) -> str:
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
    return (
      f"Session summary already exists: {memory_id}.\n"
      f"-> No action needed. The summary is already indexed and searchable."
    )

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
  try:
    _rotate_session_summaries(repo_dir, max_keep=3)
  except Exception:
    logger.warning("Session summary rotation failed.")

  git_sync(get_rag_dir(), f"session-summary: {memory_id}")
  return (
    f"Saved session summary {memory_id}. Embedded and indexed for vector search.\n"
    f"-> Session context preserved for next time."
  )


def main() -> None:
  """Run the MCP server with stdio transport."""
  mcp.run(transport="stdio")


if __name__ == "__main__":
  main()
