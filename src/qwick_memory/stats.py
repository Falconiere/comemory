"""Usage statistics for qwick-memory: retrieval counts and feedback tracking."""

from __future__ import annotations

import json
import logging
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

logger = logging.getLogger(__name__)


def load_stats(stats_path: Path | None = None) -> dict[str, Any]:
  """Load stats from JSON file. Returns empty dict on missing/corrupted file."""
  if stats_path is None:
    from qwick_memory.config import get_stats_path

    stats_path = get_stats_path()
  if not stats_path.exists():
    return {}
  try:
    return json.loads(stats_path.read_text())
  except (json.JSONDecodeError, OSError):
    logger.warning("Could not read stats file %s; returning empty stats.", stats_path)
    return {}


def save_stats(data: dict[str, Any], stats_path: Path | None = None) -> None:
  """Atomically write stats to JSON file (temp file then rename)."""
  if stats_path is None:
    from qwick_memory.config import get_stats_path

    stats_path = get_stats_path()
  stats_path.parent.mkdir(parents=True, exist_ok=True)
  tmp = stats_path.with_suffix(".tmp")
  try:
    tmp.write_text(json.dumps(data, indent=2))
    tmp.rename(stats_path)
  except OSError:
    logger.warning("Failed to write stats file %s", stats_path)
    tmp.unlink(missing_ok=True)


def increment_retrieval(memory_ids: list[str], stats_path: Path | None = None) -> None:
  """Increment retrieval_count for each memory ID."""
  stats = load_stats(stats_path)
  now = datetime.now(timezone.utc).isoformat()
  for mid in memory_ids:
    if mid not in stats:
      stats[mid] = {"retrieval_count": 0, "usage_count": 0, "last_retrieved": now}
    stats[mid]["retrieval_count"] += 1
    stats[mid]["last_retrieved"] = now
  save_stats(stats, stats_path)


def record_feedback(
  used_ids: list[str],
  irrelevant_ids: list[str],
  stats_path: Path | None = None,
) -> None:
  """Record which memories were used vs irrelevant."""
  stats = load_stats(stats_path)
  for mid in used_ids:
    if mid not in stats:
      stats[mid] = {"retrieval_count": 1, "usage_count": 0, "last_retrieved": ""}
    stats[mid]["usage_count"] += 1
  save_stats(stats, stats_path)
