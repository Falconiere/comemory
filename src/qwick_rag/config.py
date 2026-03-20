# src/qwick_rag/config.py
"""Shared helpers: resolve paths, get repo/author/index. Used by CLI + MCP server."""

import os
from pathlib import Path

from qwick_rag.git_utils import detect_author, detect_repo_name


def get_rag_dir() -> Path:
  """Resolve the qwick-rag root directory."""
  env = os.environ.get("QWICK_RAG_DIR")
  if env:
    return Path(env)
  return Path.cwd()


def get_memories_dir() -> Path:
  return get_rag_dir() / "memories"


def get_vectordb_dir() -> Path:
  return get_rag_dir() / ".vectordb"


def get_repo() -> str:
  env = os.environ.get("QWICK_RAG_REPO")
  if env:
    return env
  return detect_repo_name()


def get_author() -> str:
  env = os.environ.get("QWICK_RAG_AUTHOR")
  if env:
    return env
  return detect_author()


def get_index():
  """Lazy import to avoid circular dependency."""
  from qwick_rag.index import MemoryIndex

  return MemoryIndex(vectordb_dir=get_vectordb_dir())
