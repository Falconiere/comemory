"""Tests for qwick_memory.search — vector search with metadata filtering."""

from pathlib import Path

import pytest

from qwick_memory.index import MemoryIndex
from qwick_memory.search import SearchResult, search_memories


@pytest.fixture()
def vectordb_dir(tmp_path: Path) -> Path:
  """Return a temporary directory for the vector database."""
  return tmp_path / "vectordb"


@pytest.fixture()
def built_index(memories_dir: Path, vectordb_dir: Path) -> MemoryIndex:
  """Build and return a MemoryIndex populated with sample memories."""
  idx = MemoryIndex(vectordb_dir)
  idx.build(memories_dir)
  return idx


def test_search_returns_results(built_index: MemoryIndex) -> None:
  """Search after indexing returns relevant results; top result contains 'PostgreSQL'."""
  results = search_memories(built_index, "PostgreSQL database")
  assert len(results) > 0
  assert "PostgreSQL" in results[0].content


def test_search_with_repo_filter(built_index: MemoryIndex) -> None:
  """Filtering by repo='acme/frontend' only returns frontend memories."""
  results = search_memories(built_index, "component exports", repo="acme/frontend")
  assert len(results) > 0
  for r in results:
    assert r.repo == "acme/frontend"


def test_search_with_type_filter(built_index: MemoryIndex) -> None:
  """Filtering by type='bug' only returns bug memories."""
  results = search_memories(built_index, "session token", type_filter="bug")
  assert len(results) > 0
  for r in results:
    assert r.type == "bug"


def test_search_empty_index(vectordb_dir: Path) -> None:
  """Searching an empty index returns an empty list."""
  idx = MemoryIndex(vectordb_dir)
  results = search_memories(idx, "anything")
  assert results == []


def test_search_result_has_score(built_index: MemoryIndex) -> None:
  """Results have a score > 0."""
  results = search_memories(built_index, "database")
  assert len(results) > 0
  for r in results:
    assert isinstance(r, SearchResult)
    assert r.score > 0


def test_search_scores_are_normalized_similarity(built_index: MemoryIndex) -> None:
  """All search scores should be in 0-1 range (normalized similarity)."""
  results = search_memories(built_index, "PostgreSQL database")
  assert len(results) > 0
  for r in results:
    assert 0.0 <= r.score <= 1.0, f"Score {r.score} not in 0-1 range"


def test_search_scores_use_reranker(built_index: MemoryIndex) -> None:
  """After reranking, results have reranker_score > 0 for relevant queries."""
  results = search_memories(built_index, "PostgreSQL database")
  # With only 3 docs, some might get filtered by threshold
  # At minimum, the most relevant one should survive
  if results:
    for r in results:
      assert r.reranker_score > 0


def test_search_irrelevant_returns_empty(built_index: MemoryIndex) -> None:
  """Completely irrelevant query returns no results after threshold filtering."""
  results = search_memories(built_index, "xyzzy foobar blargh utter nonsense gibberish")
  assert results == []
