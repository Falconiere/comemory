"""Tests for qwick_rag.server — MCP tool functions (direct calls, no protocol)."""

import os
import tempfile
from unittest.mock import patch

import pytest


@pytest.fixture()
def rag_env():
  """Set up a temp directory and env vars for MCP tool tests."""
  with tempfile.TemporaryDirectory() as tmp:
    env = {
      "QWICK_RAG_DIR": tmp,
      "QWICK_RAG_REPO": "test/mcp-repo",
      "QWICK_RAG_AUTHOR": "mcp-tester",
    }
    with patch.dict(os.environ, env):
      os.makedirs(os.path.join(tmp, "memories"), exist_ok=True)
      yield tmp


@pytest.mark.asyncio
async def test_qwick_memory_save(rag_env: str) -> None:
  """qwick_memory_save creates a memory and returns 'Saved' in result."""
  from qwick_rag.server import qwick_memory_save

  result = await qwick_memory_save("MCP server test memory")
  assert "Saved" in result


@pytest.mark.asyncio
async def test_qwick_memory_search(rag_env: str) -> None:
  """qwick_memory_save then qwick_memory_search finds the saved content."""
  from qwick_rag.server import qwick_memory_save, qwick_memory_search

  await qwick_memory_save("PostgreSQL is great for JSONB queries")
  result = await qwick_memory_search("PostgreSQL")
  assert "PostgreSQL" in result


@pytest.mark.asyncio
async def test_qwick_memory_index(rag_env: str) -> None:
  """qwick_memory_index on empty dir returns 'Indexed' in result."""
  from qwick_rag.server import qwick_memory_index

  result = await qwick_memory_index()
  assert "Indexed" in result
