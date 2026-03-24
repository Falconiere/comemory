"""Tests for qwick_memory.memory module."""

from datetime import datetime
from pathlib import Path

import pytest

from qwick_memory.errors import MemoryParseError, StorageError
from qwick_memory.memory import Memory, generate_id, parse_memory, scan_memories, write_memory


def test_generate_id_is_deterministic():
  """Same content always produces the same ID."""
  content = "This is some memory content."
  assert generate_id(content) == generate_id(content)


def test_generate_id_is_12_hex_chars():
  """Generated ID is exactly 12 hexadecimal characters."""
  id_ = generate_id("any content here")
  assert len(id_) == 12
  assert all(c in "0123456789abcdef" for c in id_)


def test_generate_id_differs_for_different_content():
  """Different content produces different IDs."""
  id_a = generate_id("content A")
  id_b = generate_id("content B")
  assert id_a != id_b


def test_memory_dataclass():
  """Memory dataclass holds all expected fields correctly."""
  created = datetime(2026, 3, 20, 12, 0, 0)
  mem = Memory(
    id="abc123def456",
    repo=["owner/repo"],
    type="decision",
    tags=["python", "architecture"],
    author="alice",
    created=created,
    content="We decided to use FastAPI.",
  )
  assert mem.id == "abc123def456"
  assert mem.repo == ["owner/repo"]
  assert mem.type == "decision"
  assert mem.tags == ["python", "architecture"]
  assert mem.author == "alice"
  assert mem.created == created
  assert mem.content == "We decided to use FastAPI."
  assert len(mem.content_hash) == 12
  assert all(c in "0123456789abcdef" for c in mem.content_hash)


def test_write_and_parse_roundtrip(tmp_path: Path):
  """Writing a Memory and parsing it back yields identical data."""
  created = datetime(2026, 3, 20, 10, 30, 0)
  mem = Memory(
    id="aabbccddeeff",
    repo=["org/project"],
    type="bug",
    tags=["critical", "auth"],
    author="bob",
    created=created,
    content="Found a null pointer in login flow.",
  )
  filepath = tmp_path / "test_memory.md"
  write_memory(mem, filepath)
  parsed = parse_memory(filepath)

  assert parsed.id == mem.id
  assert parsed.repo == mem.repo
  assert parsed.type == mem.type
  assert parsed.tags == mem.tags
  assert parsed.author == mem.author
  assert parsed.created == mem.created
  assert parsed.content == mem.content
  assert parsed.content_hash == mem.content_hash


def test_parse_memory_invalid_yaml(tmp_path: Path):
  """parse_memory raises MemoryParseError for invalid YAML frontmatter."""
  bad_file = tmp_path / "bad.md"
  bad_file.write_text("---\n: invalid: yaml: [\n---\nsome content\n")
  with pytest.raises(MemoryParseError):
    parse_memory(bad_file)


def test_write_memory_creates_frontmatter(tmp_path: Path):
  """Written file starts with '---' and contains required frontmatter fields."""
  created = datetime(2026, 1, 1, 0, 0, 0)
  mem = Memory(
    id="112233445566",
    repo=["user/repo"],
    type="note",
    tags=["docs"],
    author="carol",
    created=created,
    content="Remember to update the README.",
  )
  filepath = tmp_path / "note.md"
  write_memory(mem, filepath)

  raw = filepath.read_text()
  assert raw.startswith("---")
  assert "id:" in raw
  assert "repo:" in raw
  assert "type:" in raw
  assert "tags:" in raw
  assert "author:" in raw
  assert "created:" in raw
  assert "content_hash:" in raw


def test_session_summary_type_is_valid() -> None:
  """session-summary is a recognized memory type."""
  from qwick_memory.memory import MEMORY_TYPES

  assert "session-summary" in MEMORY_TYPES


def test_write_memory_rejects_nested_path(tmp_path: Path) -> None:
  """write_memory raises StorageError when target is in a subdirectory."""
  memories_dir = tmp_path / "memories"
  nested_dir = memories_dir / "0.1.0"
  nested_dir.mkdir(parents=True)
  mem = Memory(
    id="aabbccddeeff",
    repo=["test/repo"],
    type="note",
    tags=[],
    author="tester",
    created=datetime(2026, 1, 1),
    content="Should fail",
  )
  with pytest.raises(StorageError, match="nested"):
    write_memory(mem, nested_dir / "test.md", memories_dir=memories_dir)


def test_scan_memories_ignores_nested_files(tmp_path: Path) -> None:
  """scan_memories only returns files directly in memories_dir, not subdirectories."""
  memories_dir = tmp_path / "memories"
  memories_dir.mkdir()
  (memories_dir / "top.md").write_text("---\nid: top\n---\ntop level")
  sub = memories_dir / "subdir"
  sub.mkdir()
  (sub / "nested.md").write_text("---\nid: nested\n---\nnested content")

  results = scan_memories(memories_dir)
  assert len(results) == 1
  assert results[0].name == "top.md"
