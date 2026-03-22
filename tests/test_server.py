"""Tests for qwick_memory.server — MCP tool functions (direct calls, no protocol)."""

import os
import tempfile
from unittest.mock import patch

import pytest


@pytest.fixture()
def rag_env():
  """Set up a temp directory and env vars for MCP tool tests."""
  with tempfile.TemporaryDirectory() as tmp:
    env = {
      "QWICK_MEMORY_DIR": tmp,
      "QWICK_MEMORY_REPO": "test/mcp-repo",
      "QWICK_MEMORY_AUTHOR": "mcp-tester",
    }
    with patch.dict(os.environ, env):
      os.makedirs(os.path.join(tmp, "memories"), exist_ok=True)
      yield tmp


@pytest.mark.asyncio
async def test_qwick_memory_save(rag_env: str) -> None:
  """qwick_memory_save creates a memory and returns 'Saved' in result."""
  from qwick_memory.server import qwick_memory_save

  result = await qwick_memory_save("MCP server test memory")
  assert "Saved" in result


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
  import time
  from pathlib import Path

  from qwick_memory.server import qwick_memory_session_summary

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
  memories_dir = Path(rag_env) / "memories"
  all_files = list(memories_dir.glob("*.md"))
  # Parse and count session-summary type
  from qwick_memory.memory import parse_memory

  summaries = [f for f in all_files if parse_memory(f).type == "session-summary"]
  assert len(summaries) == 3


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


@pytest.mark.asyncio
async def test_qwick_memory_context_shows_summary_first(rag_env: str) -> None:
  """qwick_memory_context shows latest session summary before other memories."""
  from qwick_memory.server import (
    qwick_memory_context,
    qwick_memory_save,
    qwick_memory_session_summary,
  )

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


@pytest.mark.asyncio
async def test_save_response_includes_vector_hint(rag_env: str) -> None:
  """qwick_memory_save response mentions vector search indexing."""
  from qwick_memory.server import qwick_memory_save

  result = await qwick_memory_save("Test memory for hint check", type="decision")
  assert "Embedded and indexed for vector search" in result
  assert "(decision)" in result


@pytest.mark.asyncio
async def test_save_duplicate_response_hint(rag_env: str) -> None:
  """qwick_memory_save duplicate response includes 'no action needed' hint."""
  from qwick_memory.server import qwick_memory_save

  await qwick_memory_save("Duplicate hint test content")
  result = await qwick_memory_save("Duplicate hint test content")
  assert "already exists" in result
  assert "No action needed" in result


@pytest.mark.asyncio
async def test_search_results_include_similarity_hint(rag_env: str) -> None:
  """qwick_memory_search results include semantic similarity hint."""
  from qwick_memory.server import qwick_memory_save, qwick_memory_search

  await qwick_memory_save("Redis is used for caching")
  result = await qwick_memory_search("Redis caching")
  assert "Results ranked by semantic similarity" in result


@pytest.mark.asyncio
async def test_search_no_results_includes_save_hint(rag_env: str) -> None:
  """qwick_memory_search with no results hints to save later."""
  from qwick_memory.server import qwick_memory_search

  result = await qwick_memory_search("completely nonexistent topic xyz123")
  assert "save it with qwick_memory_save" in result


@pytest.mark.asyncio
async def test_index_response_includes_vector_hint(rag_env: str) -> None:
  """qwick_memory_index response mentions vector search."""
  from qwick_memory.server import qwick_memory_index

  result = await qwick_memory_index()
  assert "searchable by semantic similarity" in result


@pytest.mark.asyncio
async def test_delete_response_confirms_both_layers(rag_env: str) -> None:
  """qwick_memory_delete response confirms disk and vector index removal."""
  from qwick_memory.server import qwick_memory_delete, qwick_memory_save

  result = await qwick_memory_save("Memory to delete for hint test")
  # Format: "Saved memory {id} ..." — ID is the third word
  memory_id = result.split()[2]

  result = await qwick_memory_delete(memory_id)
  assert "Removed from disk and vector index" in result


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
