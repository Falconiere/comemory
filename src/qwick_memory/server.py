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
from qwick_memory.search import SearchResult, search_memories

logging.basicConfig(stream=sys.stderr, level=logging.INFO)
logger = logging.getLogger(__name__)

# Eagerly validate that lancedb is importable — fail fast with diagnostics
# instead of a mysterious "No module named 'lancedb'" on first tool call.
try:
  import lancedb as _lancedb

  logger.info("lancedb %s loaded OK", _lancedb.__version__)
except ImportError:
  logger.error(
    "FATAL: Cannot import lancedb. Diagnostics:\n"
    "  sys.executable = %s\n"
    "  sys.path = %s\n"
    "  VIRTUAL_ENV = %s\n"
    "  PYTHONPATH = %s",
    sys.executable,
    sys.path,
    __import__("os").environ.get("VIRTUAL_ENV", "<not set>"),
    __import__("os").environ.get("PYTHONPATH", "<not set>"),
  )
  raise

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

**Step 4: Rate quality when saving**
When calling qwick_memory_save, rate quality 1-5:
- Specificity: names concrete files, functions, versions? (1=vague, 5=precise)
- Actionability: someone can act on this? (1=trivia, 5=directly useful)
- Context-independence: makes sense in 6 months? (1=needs conversation, 5=self-contained)
Average the three, round to nearest integer. When unsure, default to 3.

**Step 5: Give feedback after search**
After responding to a message where you used qwick_memory_search results:
- Call qwick_memory_feedback with IDs you referenced (used_ids)
  and IDs that were noise (irrelevant_ids).
- Only call once per response, not per result.
- Skip if you didn't use search in this response.
"""

mcp = FastMCP("qwick-memory", instructions=PROTOCOL)

# Token budget and tier thresholds for search results
SEARCH_TOKEN_BUDGET = 4000
CONTEXT_TOKEN_BUDGET = 6000
CONTEXT_SUMMARY_BUDGET = 2000
HIGH_RELEVANCE_THRESHOLD = 0.6
MODERATE_RELEVANCE_THRESHOLD = 0.35


def _estimate_tokens(text: str) -> int:
  """Rough token estimate: ~4 chars per token for English text."""
  return len(text) // 4


def _format_tiered_results(
  results: list[SearchResult],
  budget: int = SEARCH_TOKEN_BUDGET,
) -> str:
  """Format search results into tiered markdown with token budget."""
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
      first_sentence = r.content.split("\n")[0].split(".")[0][:80]
      header = f"**[{r.type}] {first_sentence}** — {repos} (tags: {r.tags})"
      entry = f"{header}\n{r.content}"
      cost = _estimate_tokens(entry)
      if cost > remaining:
        header_chars = len(header) + 1  # +1 for newline
        max_content_chars = max(0, (remaining * 4) - header_chars)
        entry = f"{header}\n{r.content[:max_content_chars]}... [truncated]"
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


@mcp.tool()
async def qwick_memory_save(
  content: str, type: str = "note", tags: str = "", repo: str = "", quality: int = 3
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
    quality: Quality rating 1-5. Specificity (names concrete things?),
             actionability (can act on it?), context-independence (6 months?).
             Average, round to nearest integer. Default 3.

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

  # Repo is required — no auto-detection fallback
  if not repo or not repo.strip():
    return (
      "Error: repo is required. Specify which repo(s) this memory belongs to "
      "(e.g. repo='sidegig-api' or repo='sidegig-api,sidegig-web')."
    )
  repo_list = [r.strip() for r in repo.split(",") if r.strip()]

  author = get_author()

  memories_dir = get_memories_dir()
  memories_dir.mkdir(parents=True, exist_ok=True)

  final_path = memories_dir / f"{memory_id}.md"

  if final_path.exists():
    return (
      f"Memory already exists: {memory_id}. Content was previously saved.\n"
      f"-> No action needed. The memory is already indexed and searchable."
    )

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

  tmp_path = memories_dir / f".{memory_id}.tmp"
  try:
    write_memory(memory, tmp_path, memories_dir=memories_dir)
    idx = get_index()
    idx.upsert(memory)
    tmp_path.rename(final_path)
  except Exception as exc:
    tmp_path.unlink(missing_ok=True)
    logger.exception("Failed to save memory %s", memory_id)
    return f"Error saving memory: {exc}"

  git_sync(get_rag_dir(), f"save: {memory_id} ({type})")
  repos_str = ", ".join(repo_list)
  return (
    f"Saved memory {memory_id} ({type}) for [{repos_str}]. "
    f"Embedded and indexed for vector search.\n"
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
  If you're about to answer from general knowledge, STOP — search first.
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
      "No results found.\n"
      "-> If you learn something new about this topic during this task, "
      "save it before the session ends."
    )

  result_text = _format_tiered_results(results)
  count = len(results)
  return (
    f"{count} result(s) found. Use these to inform your response — do NOT ignore them.\n\n"
    f"{result_text}"
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

    if repo and repo not in mem.repo:
      continue
    if type and mem.type != type:
      continue

    preview = mem.content[:60] + "..." if len(mem.content) > 60 else mem.content
    tag_str = ", ".join(mem.tags) if mem.tags else ""
    repo_str = ", ".join(mem.repo)
    lines.append(f"{mem.id} | {repo_str} | {mem.type} | [{tag_str}] | {preview}")

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

  matches = list(memories_dir.glob(f"{memory_id}.md"))
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

  # If no repo detected/provided, return all memories (no repo filter)
  summaries: list[Memory] = []
  regular: list[Memory] = []
  for fp in md_files:
    try:
      mem = parse_memory(fp)
    except Exception:
      continue
    if target_repo is not None and target_repo not in mem.repo:
      continue
    if mem.type == "session-summary":
      summaries.append(mem)
    else:
      regular.append(mem)

  if not summaries and not regular:
    return f"No memories found for repo: {target_repo}"

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


def _rotate_session_summaries(memories_dir: Path, repo: str, max_keep: int = 3) -> None:
  """Delete old session summaries for a repo, keeping only the most recent `max_keep`."""
  summaries: list[tuple[datetime, Path]] = []
  for fp in memories_dir.glob("*.md"):
    try:
      mem = parse_memory(fp)
      if mem.type == "session-summary" and repo in mem.repo:
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
  repo: str = "",
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
    repo: Comma-separated repo names. REQUIRED — always specify which repo(s).

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

  if not repo or not repo.strip():
    return (
      "Error: repo is required. Specify which repo(s) this session summary belongs to "
      "(e.g. repo='sidegig-api' or repo='sidegig-api,sidegig-web')."
    )
  repo_list = [r.strip() for r in repo.split(",") if r.strip()]

  author = get_author()

  memories_dir = get_memories_dir()
  memories_dir.mkdir(parents=True, exist_ok=True)

  final_path = memories_dir / f"{memory_id}.md"

  if final_path.exists():
    return (
      f"Session summary already exists: {memory_id}.\n"
      f"-> No action needed. The summary is already indexed and searchable."
    )

  memory = Memory(
    id=memory_id,
    repo=repo_list,
    type="session-summary",
    tags=["session-summary"],
    author=author,
    created=datetime.now(timezone.utc),
    content=content,
    quality=3,
  )

  tmp_path = memories_dir / f".{memory_id}.tmp"
  try:
    write_memory(memory, tmp_path, memories_dir=memories_dir)
    idx = get_index()
    idx.upsert(memory)
    tmp_path.rename(final_path)
  except Exception as exc:
    tmp_path.unlink(missing_ok=True)
    logger.exception("Failed to save session summary %s", memory_id)
    return f"Error saving session summary: {exc}"

  # Rotation: keep only 3 most recent session summaries per repo
  try:
    _rotate_session_summaries(memories_dir, repo_list[0], max_keep=3)
  except Exception:
    logger.warning("Session summary rotation failed.")

  git_sync(get_rag_dir(), f"session-summary: {memory_id}")
  return (
    f"Saved session summary {memory_id}. Embedded and indexed for vector search.\n"
    f"-> Session context preserved for next time."
  )


@mcp.tool()
async def qwick_memory_feedback(used_ids: str = "", irrelevant_ids: str = "") -> str:
  """Report which search results were useful after responding.

  Call this AFTER responding to a message where you used qwick_memory_search results.
  Only call once per response. Skip if you didn't use search in this response.

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

  try:
    from qwick_memory.config import get_search_log_path

    log_path = get_search_log_path()
    entry = {
      "timestamp": datetime.now(timezone.utc).isoformat(),
      "type": "feedback",
      "used_ids": used,
      "irrelevant_ids": irrelevant,
    }
    import json as _json

    with open(log_path, "a") as f:
      f.write(_json.dumps(entry) + "\n")
  except Exception:
    pass

  return (
    f"Recorded feedback: {len(used)} used, {len(irrelevant)} irrelevant.\n"
    f"-> This feedback improves future search ranking."
  )


def main() -> None:
  """Run the MCP server with stdio transport."""
  mcp.run(transport="stdio")


if __name__ == "__main__":
  main()
