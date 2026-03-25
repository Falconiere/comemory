"""Search pipeline for qwick-memory: vector + metadata filtering."""

from __future__ import annotations

import json
import logging
import math
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
  from qwick_memory.index import MemoryIndex

logger = logging.getLogger(__name__)

MIN_RELEVANCE_SCORE = 0.3
MAX_SCORE_GAP = 0.15

HALF_LIFE_DAYS: dict[str, int] = {
  "convention": 365,
  "preference": 365,
  "decision": 180,
  "pattern": 180,
  "discovery": 120,
  "bug": 90,
  "note": 60,
  "session-summary": 14,
}
DEFAULT_HALF_LIFE = 90


@dataclass
class SearchResult:
  """A single search result with relevance score.

  Score lifecycle: before reranking, score = raw similarity.
  After pipeline: score = composite final score (reranker * freshness * quality * usage).
  """

  id: str
  repo: str
  type: str
  tags: str
  author: str
  created: str
  content: str
  score: float
  reranker_score: float = 0.0
  quality: int = 3
  enriched_content: str = ""


def _apply_thresholds(
  results: list[SearchResult],
  min_score: float = MIN_RELEVANCE_SCORE,
  max_gap: float = MAX_SCORE_GAP,
) -> list[SearchResult]:
  """Filter results by hard floor and gap detection on reranker_score."""
  if not results:
    return []

  # Hard floor
  above = [r for r in results if r.reranker_score >= min_score]
  if not above:
    return []

  # Results should already be sorted by reranker_score descending
  above.sort(key=lambda r: r.reranker_score, reverse=True)

  # Gap detection
  filtered = [above[0]]
  for i in range(1, len(above)):
    gap = above[i - 1].reranker_score - above[i].reranker_score
    if gap > max_gap:
      break
    filtered.append(above[i])

  return filtered


def _freshness_decay(created: datetime, memory_type: str) -> float:
  """Exponential decay based on memory age and type-specific half-life."""
  half_life = HALF_LIFE_DAYS.get(memory_type, DEFAULT_HALF_LIFE)
  now = datetime.now(timezone.utc)
  if created.tzinfo is None:
    created = created.replace(tzinfo=timezone.utc)
  age_days = max(0.0, (now - created).total_seconds() / 86400)
  return math.exp(-math.log(2) / half_life * age_days)


def _compute_final_score(
  reranker_score: float,
  memory_type: str,
  created: datetime,
  quality: int,
  stats: dict[str, Any] | None,
) -> float:
  """Combine reranker score with freshness, quality, and usage signals."""
  freshness = _freshness_decay(created, memory_type)
  quality_boost = 0.6 + 0.08 * quality
  if stats is not None:
    retrieval_count = stats.get("retrieval_count", 0)
    usage_count = stats.get("usage_count", 0)
    usage_boost = 0.8 + 0.2 * (usage_count / max(1, retrieval_count))
  else:
    usage_boost = 0.9
  return reranker_score * freshness * quality_boost * usage_boost


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

  query_vector = index._embed_query(query)

  # Build metadata filter clauses (sanitize inputs to prevent injection)
  where_clauses: list[str] = []
  if repo is not None:
    safe_repo = repo.replace('"', '\\"').replace("%", "")
    where_clauses.append(f'repo LIKE "%{safe_repo}%"')
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
    return [_row_to_result(row, score_key="_relevance_score", normalize=False) for row in rows]
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
  return [_row_to_result(row, score_key="_distance", normalize=True) for row in rows]


def _row_to_result(row: dict[str, Any], score_key: str, normalize: bool = False) -> SearchResult:
  """Map a LanceDB result row to a SearchResult dataclass."""
  raw_score = float(row.get(score_key, 0.0))
  score = max(0.0, min(1.0, 1.0 - (raw_score / 2.0))) if normalize else raw_score
  return SearchResult(
    id=row["id"],
    repo=row["repo"],
    type=row["type"],
    tags=row["tags"],
    author=row["author"],
    created=row["created"],
    content=row["content"],
    score=score,
  )
