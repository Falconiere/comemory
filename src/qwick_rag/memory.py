"""Memory model for qwick-memory: core data structure for stored memories."""

import hashlib
from dataclasses import dataclass, field
from datetime import datetime
from pathlib import Path
from typing import Literal

import frontmatter
import yaml

from qwick_rag.errors import MemoryParseError

MEMORY_TYPES = (
  "decision",
  "bug",
  "convention",
  "discovery",
  "pattern",
  "preference",
  "note",
  "session-summary",
)

MemoryType = Literal[
  "decision",
  "bug",
  "convention",
  "discovery",
  "pattern",
  "preference",
  "note",
  "session-summary",
]


def generate_id(content: str) -> str:
  """Return the first 12 hex characters of the SHA-256 hash of content."""
  return hashlib.sha256(content.encode()).hexdigest()[:12]


@dataclass
class Memory:
  """A single unit of knowledge stored in qwick-memory."""

  id: str
  repo: str
  type: MemoryType
  tags: list[str]
  author: str
  created: datetime
  content: str
  content_hash: str = field(init=False)

  def __post_init__(self) -> None:
    self.content_hash = generate_id(self.content)


def write_memory(memory: Memory, filepath: Path) -> None:
  """Serialize a Memory to a markdown file with YAML frontmatter."""
  post = frontmatter.Post(
    content=memory.content,
    id=memory.id,
    repo=memory.repo,
    type=memory.type,
    tags=memory.tags,
    author=memory.author,
    created=memory.created,
    content_hash=memory.content_hash,
  )
  filepath.write_text(frontmatter.dumps(post))


def parse_memory(filepath: Path) -> Memory:
  """Parse a markdown file with YAML frontmatter into a Memory object.

  Raises:
    MemoryParseError: if the file has invalid YAML or is missing required fields.
  """
  raw = filepath.read_text()
  try:
    post = frontmatter.loads(raw)
  except yaml.YAMLError as exc:
    raise MemoryParseError(
      f"Invalid YAML frontmatter in {filepath}",
      suggested_fix="Check that the frontmatter is valid YAML.",
      context={"filepath": str(filepath), "error": str(exc)},
    ) from exc

  required = ("id", "repo", "type", "tags", "author", "created")
  missing = [key for key in required if key not in post.metadata]
  if missing:
    raise MemoryParseError(
      f"Missing required frontmatter fields in {filepath}: {missing}",
      suggested_fix="Ensure all required fields are present in the frontmatter.",
      context={"filepath": str(filepath), "missing": missing},
    )

  try:
    created_raw = post.metadata["created"]
    if isinstance(created_raw, str):
      created = datetime.fromisoformat(created_raw)
    elif isinstance(created_raw, datetime):
      created = created_raw
    else:
      msg = f"created must be a datetime or ISO string, got {type(created_raw)}"
      raise TypeError(msg)

    mem_type: MemoryType = str(post.metadata["type"])  # type: ignore[assignment]  # validated by MEMORY_TYPES check downstream
    return Memory(
      id=str(post.metadata["id"]),
      repo=str(post.metadata["repo"]),
      type=mem_type,
      tags=[str(t) for t in list(post.metadata["tags"])],  # type: ignore[arg-type]  # metadata values typed as object; runtime guarantees list
      author=str(post.metadata["author"]),
      created=created,
      content=post.content,
    )
  except (KeyError, ValueError, TypeError) as exc:
    raise MemoryParseError(
      f"Failed to parse memory fields in {filepath}: {exc}",
      suggested_fix="Check that all frontmatter fields have valid values.",
      context={"filepath": str(filepath), "error": str(exc)},
    ) from exc


def scan_memories(memories_dir: Path) -> list[Path]:
  """Return all markdown files found recursively under memories_dir."""
  return list(memories_dir.rglob("*.md"))
