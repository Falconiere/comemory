"""Search pipeline for qwick-memory: vector + metadata filtering."""

from __future__ import annotations

import json
import logging
import math
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import TYPE_CHECKING, Any

from fastembed.rerank.cross_encoder import TextCrossEncoder

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

RERANKER_MODEL = "Xenova/ms-marco-MiniLM-L-6-v2"
_reranker: TextCrossEncoder | None = None


def _get_reranker() -> TextCrossEncoder:
  """Lazy-load the cross-encoder reranker."""
  global _reranker
  if _reranker is None:
    _reranker = TextCrossEncoder(model_name=RERANKER_MODEL)
  return _reranker


def _rerank(
  query: str,
  results: list[SearchResult],
  limit: int,
) -> list[SearchResult]:
  """Rerank results using cross-encoder, normalize logits via sigmoid."""
  if not results:
    return []

  reranker = _get_reranker()
  documents = [r.content for r in results]
  raw_scores = list(reranker.rerank(query, documents))

  # Sigmoid normalize raw logits to 0-1
  for result, raw in zip(results, raw_scores, strict=True):
    result.reranker_score = 1.0 / (1.0 + math.exp(-raw))

  # Sort by reranker_score descending
  results.sort(key=lambda r: r.reranker_score, reverse=True)
  return results[:limit]


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


def _log_search(
  query: str,
  filters: dict[str, str | None],
  results: list[SearchResult],
  filtered_count: int,
) -> None:
  """Append search interaction to JSONL log. Fire-and-forget."""
  try:
    from qwick_memory.config import get_search_log_path

    log_path = get_search_log_path()
    log_path.parent.mkdir(parents=True, exist_ok=True)
    entry = {
      "timestamp": datetime.now(timezone.utc).isoformat(),
      "type": "search",
      "query": query,
      **filters,
      "results": [
        {"id": r.id, "reranker_score": round(r.reranker_score, 4), "final_score": round(r.score, 4)}
        for r in results
      ],
      "result_count": len(results),
      "filtered_count": filtered_count,
    }
    with open(log_path, "a") as f:
      f.write(json.dumps(entry) + "\n")
  except Exception:
    logger.debug("Failed to log search interaction.")


def search_memories(
  index: MemoryIndex,
  query: str,
  repo: str | None = None,
  type_filter: str | None = None,
  tag: str | None = None,
  limit: int = 10,
) -> list[SearchResult]:
  """Search the memory index with optional metadata filters.

  Pipeline: hybrid search -> cross-encoder rerank -> threshold filter -> combined scoring.
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

  # Step 1: Hybrid search (over-retrieve)
  retrieve_limit = max(limit * 2, 20)
  results = _try_hybrid_search(table, query, query_vector, where_expr, retrieve_limit)
  if results is None:
    results = _vector_search(table, query_vector, where_expr, retrieve_limit)

  if not results:
    return []

  # Step 2: Cross-encoder rerank
  results = _rerank(query, results, retrieve_limit)

  # Track pre-threshold count for logging
  pre_threshold_count = len(results)

  # Step 3: Threshold filter on reranker_score
  results = _apply_thresholds(results)

  if not results:
    return []

  # Increment retrieval counts (fire-and-forget)
  try:
    from qwick_memory.stats import increment_retrieval

    increment_retrieval([r.id for r in results])
  except Exception:
    logger.debug("Failed to increment retrieval counts.")

  # Step 4: Combined scoring
  from qwick_memory.stats import load_stats

  all_stats = load_stats()
  for r in results:
    created_dt = datetime.fromisoformat(r.created)
    mem_stats = all_stats.get(r.id)
    r.score = _compute_final_score(
      reranker_score=r.reranker_score,
      memory_type=r.type,
      created=created_dt,
      quality=r.quality,
      stats=mem_stats,
    )

  # Step 5: Sort by final_score, return top limit
  results.sort(key=lambda r: r.score, reverse=True)

  # Log search interaction (fire-and-forget)
  _log_search(
    query=query,
    filters={"repo_filter": repo, "type_filter": type_filter, "tag_filter": tag},
    results=results[:limit],
    filtered_count=pre_threshold_count - len(results),
  )

  return results[:limit]


def _try_hybrid_search(
  table: Any,
  query: str,
  query_vector: list[float],
  where_expr: str | None,
  limit: int,
) -> list[SearchResult] | None:
  """Attempt hybrid (vector + FTS) search with 50/50 vector/FTS weights."""
  try:
    from lancedb.rerankers import LinearCombinationReranker

    fuser = LinearCombinationReranker(weight=0.5)
    builder = table.search(query_type="hybrid").vector(query_vector).text(query)
    builder = builder.rerank(reranker=fuser)
    if where_expr:
      builder = builder.where(where_expr)
    builder = builder.limit(limit)
    rows = builder.to_list()
    return [_row_to_result(row, score_key="_relevance_score", normalize=False) for row in rows]
  except Exception:
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
    quality=row.get("quality", 3),
    enriched_content=row.get("enriched_content", ""),
  )
