"""Shared helpers: resolve paths, get repo/author/index. Used by CLI + MCP server."""

import os
from pathlib import Path

from qwick_memory.git_utils import detect_author, detect_repo_name


def get_rag_dir() -> Path:
  """Resolve the qwick-memory root directory.

  Priority: QWICK_MEMORY_DIR env var > ~/.qwick-memory/ (global default).
  """
  env = os.environ.get("QWICK_MEMORY_DIR")
  if env:
    return Path(env)
  return Path.home() / ".qwick-memory"


def get_memories_dir() -> Path:
  return get_rag_dir() / "memories"


def get_vectordb_dir() -> Path:
  return get_rag_dir() / ".vectordb"


def get_repo() -> str | None:
  """Return repo name from env var or git detection. None if undetectable."""
  env = os.environ.get("QWICK_MEMORY_REPO")
  if env:
    return env
  return detect_repo_name()


def get_author() -> str:
  env = os.environ.get("QWICK_MEMORY_AUTHOR")
  if env:
    return env
  return detect_author()


def get_index():
  """Lazy import to avoid circular dependency."""
  from qwick_memory.index import MemoryIndex

  return MemoryIndex(vectordb_dir=get_vectordb_dir())


def get_stats_path() -> Path:
  return get_rag_dir() / ".stats.json"


def get_search_log_path() -> Path:
  return get_rag_dir() / ".search_log.jsonl"


def _env_float(name: str, default: float) -> float:
  raw = os.environ.get(name)
  if raw is None or raw == "":
    return default
  try:
    return float(raw)
  except ValueError:
    return default


MIN_RELEVANCE_SCORE: float = _env_float("QWICK_MEMORY_MIN_RELEVANCE", 0.3)
MAX_SCORE_GAP: float = _env_float("QWICK_MEMORY_MAX_GAP", 0.15)
HYBRID_WEIGHT: float = _env_float("QWICK_MEMORY_HYBRID_WEIGHT", 0.5)
RERANKER_MODEL: str = os.environ.get("QWICK_MEMORY_RERANKER_MODEL", "Xenova/ms-marco-MiniLM-L-6-v2")


def get_search_events_path() -> Path:
  """Append-only event log for stats (compacted into .stats.json)."""
  return get_rag_dir() / ".stats.events.jsonl"
