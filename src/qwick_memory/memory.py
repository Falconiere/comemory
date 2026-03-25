"""Memory model for qwick-memory: core data structure for stored memories."""

import hashlib
import logging
from dataclasses import dataclass, field
from datetime import datetime
from pathlib import Path
from typing import Literal

import frontmatter
import yaml

from qwick_memory.errors import MemoryParseError, StorageError

logger = logging.getLogger(__name__)

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
  repo: list[str]
  type: MemoryType
  tags: list[str]
  author: str
  created: datetime
  content: str
  quality: int = 3
  content_hash: str = field(init=False)

  def __post_init__(self) -> None:
    self.content_hash = generate_id(self.content)


def write_memory(memory: Memory, filepath: Path, memories_dir: Path | None = None) -> None:
  """Serialize a Memory to a markdown file with YAML frontmatter."""
  if memories_dir is not None and filepath.parent.resolve() != memories_dir.resolve():
    raise StorageError(
      f"Cannot write to nested path: {filepath}",
      suggested_fix="Write directly to the memories/ directory, not a subdirectory.",
      context={"filepath": str(filepath)},
    )
  post = frontmatter.Post(
    content=memory.content,
    id=memory.id,
    repo=memory.repo,
    type=memory.type,
    tags=memory.tags,
    author=memory.author,
    created=memory.created,
    quality=memory.quality,
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
    # repo: accept both old string format and new list format for backwards compat
    raw_repo = post.metadata["repo"]
    if isinstance(raw_repo, str):
      repo_list = [raw_repo]
    elif isinstance(raw_repo, list):
      repo_list = [str(r) for r in raw_repo]
    else:
      repo_list = [str(raw_repo)]
    quality = int(post.metadata.get("quality", 3))
    return Memory(
      id=str(post.metadata["id"]),
      repo=repo_list,
      type=mem_type,
      tags=[str(t) for t in list(post.metadata["tags"])],  # type: ignore[arg-type]  # metadata values typed as object; runtime guarantees list
      author=str(post.metadata["author"]),
      created=created,
      content=post.content,
      quality=quality,
    )
  except (KeyError, ValueError, TypeError) as exc:
    raise MemoryParseError(
      f"Failed to parse memory fields in {filepath}: {exc}",
      suggested_fix="Check that all frontmatter fields have valid values.",
      context={"filepath": str(filepath), "error": str(exc)},
    ) from exc


def scan_memories(memories_dir: Path) -> list[Path]:
  """Return all markdown files directly in memories_dir (flat layout only)."""
  subdirs = [p for p in memories_dir.iterdir() if p.is_dir()] if memories_dir.exists() else []
  if subdirs:
    logger.warning(
      "Found subdirectories in memories/: %s. Flat layout expected — these will be ignored.",
      [d.name for d in subdirs],
    )
  return list(memories_dir.glob("*.md"))
