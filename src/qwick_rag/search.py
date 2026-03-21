"""Search pipeline for qwick-rag: vector + metadata filtering."""

from __future__ import annotations

import logging
from dataclasses import dataclass
from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
  from qwick_rag.index import MemoryIndex

logger = logging.getLogger(__name__)


@dataclass
class SearchResult:
  """A single search result with relevance score."""

  id: str
  repo: str
  type: str
  tags: str
  author: str
  created: str
  content: str
  score: float


def search_memories(
  index: MemoryIndex,
  query: str,
  repo: str | None = None,
  type_filter: str | None = None,
  tag: str | None = None,
  limit: int = 10,
) -> list[SearchResult]:
  """Search the memory index with optional metadata filters.

  Attempts hybrid search (vector + full-text) first, falling back to
  vector-only search if the FTS index is unavailable.
  """
  table = index._get_table()
  if table is None:
    return []

  query_vector = index._embed([query])[0]

  # Build metadata filter clauses (sanitize inputs to prevent injection)
  where_clauses: list[str] = []
  if repo is not None:
    safe_repo = repo.replace('"', '\\"')
    where_clauses.append(f'repo = "{safe_repo}"')
  if type_filter is not None:
    safe_type = type_filter.replace('"', '\\"')
    where_clauses.append(f'type = "{safe_type}"')
  if tag is not None:
    safe_tag = tag.replace('"', '\\"').replace("%", "")
    where_clauses.append(f'tags LIKE "%{safe_tag}%"')

  where_expr = " AND ".join(where_clauses) if where_clauses else None

  # Try hybrid search first, fall back to vector-only
  results = _try_hybrid_search(table, query, query_vector, where_expr, limit)
  if results is not None:
    return results

  return _vector_search(table, query_vector, where_expr, limit)


def _try_hybrid_search(
  table: Any,
  query: str,
  query_vector: list[float],
  where_expr: str | None,
  limit: int,
) -> list[SearchResult] | None:
  """Attempt hybrid (vector + FTS) search. Returns None on failure."""
  try:
    builder = table.search(query_type="hybrid").vector(query_vector).text(query)
    if where_expr:
      builder = builder.where(where_expr)
    builder = builder.limit(limit)
    rows = builder.to_list()
    return [_row_to_result(row, score_key="_relevance_score") for row in rows]
  except Exception:
    # Hybrid search may not be available (no FTS index, API mismatch, etc.)
    # This is acceptable for MVP — fall back to vector-only.
    logger.debug("Hybrid search failed; falling back to vector-only search.")
    return None


def _vector_search(
  table: Any,
  query_vector: list[float],
  where_expr: str | None,
  limit: int,
) -> list[SearchResult]:
  """Pure vector (ANN) search."""
  builder = table.search(query_vector)
  if where_expr:
    builder = builder.where(where_expr)
  builder = builder.limit(limit)
  rows = builder.to_list()
  return [_row_to_result(row, score_key="_distance") for row in rows]


def _row_to_result(row: dict[str, Any], score_key: str) -> SearchResult:
  """Map a LanceDB result row to a SearchResult dataclass."""
  return SearchResult(
    id=row["id"],
    repo=row["repo"],
    type=row["type"],
    tags=row["tags"],
    author=row["author"],
    created=row["created"],
    content=row["content"],
    score=float(row.get(score_key, 0.0)),
  )
