"""Tests for qwick_memory.index — LanceDB indexing with incremental rebuild."""

import json
from datetime import datetime
from pathlib import Path

import pytest

from qwick_memory.index import MemoryIndex
from qwick_memory.memory import Memory, write_memory


@pytest.fixture()
def vectordb_dir(tmp_path: Path) -> Path:
  """Return a temporary directory for the vector database."""
  return tmp_path / "vectordb"


def test_build_index_from_memories(
  memories_dir: Path,
  vectordb_dir: Path,
) -> None:
  """Building the index from 3 memory files yields stats new=3."""
  idx = MemoryIndex(vectordb_dir)
  stats = idx.build(memories_dir)

  assert stats["new"] == 3
  assert stats["updated"] == 0
  assert stats["deleted"] == 0
  assert idx.count() == 3


def test_incremental_index_skips_unchanged(
  memories_dir: Path,
  vectordb_dir: Path,
) -> None:
  """A second build with no changes skips everything (new=0)."""
  idx = MemoryIndex(vectordb_dir)
  idx.build(memories_dir)

  stats = idx.build(memories_dir)
  assert stats["new"] == 0
  assert stats["updated"] == 0
  assert stats["deleted"] == 0
  assert idx.count() == 3


def test_index_detects_new_file(
  memories_dir: Path,
  vectordb_dir: Path,
) -> None:
  """Adding a 4th memory file is detected as new=1."""
  idx = MemoryIndex(vectordb_dir)
  idx.build(memories_dir)

  new_mem = Memory(
    id="mem_new_004",
    repo=["acme/backend"],
    type="discovery",
    tags=["performance"],
    author="dave",
    created=datetime(2026, 3, 1, 12, 0, 0),
    content="Connection pooling with PgBouncer reduced p99 latency by 40%.",
  )
  write_memory(new_mem, memories_dir / "mem_new_004.md")

  stats = idx.build(memories_dir)
  assert stats["new"] == 1
  assert stats["updated"] == 0
  assert stats["deleted"] == 0
  assert idx.count() == 4


def test_index_detects_deleted_file(
  memories_dir: Path,
  vectordb_dir: Path,
) -> None:
  """Deleting a memory file is detected as deleted=1."""
  idx = MemoryIndex(vectordb_dir)
  idx.build(memories_dir)

  # Remove one file
  (memories_dir / "mem_pg_001.md").unlink()

  stats = idx.build(memories_dir)
  assert stats["deleted"] == 1
  assert idx.count() == 2


def test_force_rebuild(
  memories_dir: Path,
  vectordb_dir: Path,
) -> None:
  """force=True re-indexes all memories, new=3."""
  idx = MemoryIndex(vectordb_dir)
  idx.build(memories_dir)

  stats = idx.build(memories_dir, force=True)
  assert stats["new"] == 3
  assert stats["updated"] == 0
  assert stats["deleted"] == 0
  assert idx.count() == 3


def test_embed_documents_adds_prefix(vectordb_dir: Path) -> None:
  """_embed_documents prepends 'search_document: ' prefix to texts."""
  idx = MemoryIndex(vectordb_dir)
  # Embed same text with and without prefix — vectors should differ
  doc_vecs = idx._embed_documents(["hello world"])
  query_vecs = idx._embed_query("hello world")
  assert doc_vecs[0] != query_vecs


def test_upsert_single_memory(
  vectordb_dir: Path,
) -> None:
  """Upserting a single memory creates exactly 1 row."""
  idx = MemoryIndex(vectordb_dir)
  mem = Memory(
    id="mem_upsert_001",
    repo=["acme/backend"],
    type="note",
    tags=["testing"],
    author="eve",
    created=datetime(2026, 3, 10, 8, 0, 0),
    content="Always seed the test database with factory fixtures.",
  )

  idx.upsert(mem)
  assert idx.count() == 1


def test_meta_not_overwritten_on_init(tmp_path: Path) -> None:
  """MemoryIndex.__init__ should NOT overwrite meta.json with current model."""
  vectordb_dir = tmp_path / "vectordb"
  vectordb_dir.mkdir()
  meta_path = vectordb_dir / "meta.json"
  meta_path.write_text(json.dumps({"model": "old-model-name"}))

  MemoryIndex(vectordb_dir)

  meta = json.loads(meta_path.read_text())
  assert meta["model"] == "old-model-name", "init should not overwrite existing meta.json"


def test_model_matches_detects_mismatch(tmp_path: Path) -> None:
  """model_matches() returns False when meta.json has a different model."""
  vectordb_dir = tmp_path / "vectordb"
  vectordb_dir.mkdir()
  meta_path = vectordb_dir / "meta.json"
  meta_path.write_text(json.dumps({"model": "old-model-name"}))

  idx = MemoryIndex(vectordb_dir)
  assert idx.model_matches() is False
