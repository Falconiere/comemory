from qwick_rag.errors import (
  ConfigError,
  GitError,
  MemoryParseError,
  QwickRagError,
  StorageError,
  VectorIndexError,
)


def test_error_hierarchy():
  """All custom errors inherit from QwickRagError."""
  assert issubclass(StorageError, QwickRagError)
  assert issubclass(VectorIndexError, QwickRagError)
  assert issubclass(GitError, QwickRagError)
  assert issubclass(MemoryParseError, QwickRagError)
  assert issubclass(ConfigError, QwickRagError)


def test_error_carries_context():
  """Errors carry message, suggested_fix, and context."""
  err = StorageError(
    "Cannot write file",
    suggested_fix="Check disk space",
    context={"path": "/tmp/foo.md"},
  )
  assert str(err) == "Cannot write file"
  assert err.suggested_fix == "Check disk space"
  assert err.context["path"] == "/tmp/foo.md"
