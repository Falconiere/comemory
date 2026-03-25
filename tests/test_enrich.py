"""Tests for document enrichment — pure function, no MemoryIndex needed."""

from datetime import datetime

from qwick_memory.index import _enrich_text
from qwick_memory.memory import Memory


def test_enrich_text_basic():
  """Enriched text includes repo, type, tags, and content."""
  mem = Memory(
    id="e001",
    repo=["sidegig-api"],
    type="bug",
    tags=["database", "postgres"],
    author="alice",
    created=datetime(2026, 1, 1),
    content="Connection pool timeout.",
  )
  result = _enrich_text(mem)
  assert "[Repository: sidegig-api]" in result
  assert "[Type: bug]" in result
  assert "[Tags: database, postgres]" in result
  assert "Connection pool timeout." in result


def test_enrich_text_empty_tags():
  """Empty tags omits the [Tags: ] block entirely."""
  mem = Memory(
    id="e002",
    repo=["test"],
    type="note",
    tags=[],
    author="alice",
    created=datetime(2026, 1, 1),
    content="No tags here.",
  )
  result = _enrich_text(mem)
  assert "[Tags:" not in result
  assert "No tags here." in result


def test_enrich_text_multi_repo():
  """Multiple repos are comma-separated."""
  mem = Memory(
    id="e003",
    repo=["api", "web"],
    type="decision",
    tags=["arch"],
    author="alice",
    created=datetime(2026, 1, 1),
    content="Shared config.",
  )
  result = _enrich_text(mem)
  assert "[Repository: api, web]" in result
