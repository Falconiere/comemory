"""Structured error types for qwick-memory."""

from typing import Any


class QwickRagError(Exception):
  """Base error for all qwick-memory errors."""

  def __init__(
    self,
    message: str,
    suggested_fix: str = "",
    context: dict[str, Any] | None = None,
  ):
    super().__init__(message)
    self.suggested_fix = suggested_fix
    self.context = context or {}


class StorageError(QwickRagError):
  """File system issues (permissions, disk full, path not found)."""


class VectorIndexError(QwickRagError):
  """LanceDB issues (corrupt DB, embedding failures)."""


class GitError(QwickRagError):
  """Git detection failures (no remote, no user config)."""


class MemoryParseError(QwickRagError):
  """Malformed frontmatter, invalid YAML, missing required fields."""


class ConfigError(QwickRagError):
  """Invalid config, missing dependencies."""
