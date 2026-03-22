"""Shared test fixtures for qwick-rag tests."""

from datetime import datetime
from pathlib import Path

import pytest

import qwick_rag.git_utils as _git_mod
from qwick_rag.memory import Memory, write_memory


@pytest.fixture(autouse=True)
def _reset_git_cache(monkeypatch: pytest.MonkeyPatch) -> None:
  """Reset git_sync cache and prevent tests from discovering the real remote."""
  _git_mod._rag_repo_ready = None
  monkeypatch.setattr(_git_mod, "_find_source_repo", lambda: None)


@pytest.fixture()
def sample_memories() -> list[Memory]:
  """Three sample memories used across index tests."""
  return [
    Memory(
      id="mem_pg_001",
      repo="acme/backend",
      type="decision",
      tags=["database", "postgresql"],
      author="alice",
      created=datetime(2026, 1, 15, 10, 0, 0),
      content=(
        "We chose PostgreSQL as the primary database for its JSONB support and strong ecosystem."
      ),
    ),
    Memory(
      id="mem_sess_002",
      repo="acme/backend",
      type="bug",
      tags=["auth", "session"],
      author="bob",
      created=datetime(2026, 2, 1, 14, 30, 0),
      content=(
        "Session tokens were not being invalidated on logout due to a missing Redis DEL call."
      ),
    ),
    Memory(
      id="mem_react_003",
      repo="acme/frontend",
      type="convention",
      tags=["react", "exports"],
      author="carol",
      created=datetime(2026, 2, 20, 9, 0, 0),
      content=(
        "All React components must use named exports, not default exports, for better tree-shaking."
      ),
    ),
  ]


@pytest.fixture()
def memories_dir(tmp_path: Path, sample_memories: list[Memory]) -> Path:
  """Write sample memories to markdown files in a temp directory."""
  mem_dir = tmp_path / "memories"
  mem_dir.mkdir()
  for mem in sample_memories:
    filepath = mem_dir / f"{mem.id}.md"
    write_memory(mem, filepath)
  return mem_dir
