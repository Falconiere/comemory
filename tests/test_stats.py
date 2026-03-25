"""Tests for qwick_memory.stats — usage tracking with atomic writes."""

from qwick_memory.stats import (
  increment_retrieval,
  load_stats,
  record_feedback,
  save_stats,
)


def test_load_stats_missing_file(tmp_path):
  """Loading from non-existent file returns empty dict."""
  stats = load_stats(tmp_path / "nope.json")
  assert stats == {}


def test_load_stats_corrupted_file(tmp_path):
  """Loading from corrupted file returns empty dict."""
  path = tmp_path / "bad.json"
  path.write_text("not json {{{")
  stats = load_stats(path)
  assert stats == {}


def test_save_and_load_roundtrip(tmp_path):
  """Save then load returns same data."""
  path = tmp_path / "stats.json"
  data = {
    "abc123": {"retrieval_count": 5, "usage_count": 3, "last_retrieved": "2026-03-24T00:00:00"}
  }
  save_stats(data, path)
  loaded = load_stats(path)
  assert loaded == data


def test_increment_retrieval(tmp_path):
  """increment_retrieval creates entry and bumps count."""
  path = tmp_path / "stats.json"
  increment_retrieval(["id1", "id2"], path)
  stats = load_stats(path)
  assert stats["id1"]["retrieval_count"] == 1
  assert stats["id2"]["retrieval_count"] == 1

  increment_retrieval(["id1"], path)
  stats = load_stats(path)
  assert stats["id1"]["retrieval_count"] == 2


def test_record_feedback(tmp_path):
  """record_feedback increments usage_count for used_ids."""
  path = tmp_path / "stats.json"
  increment_retrieval(["id1", "id2"], path)
  record_feedback(used_ids=["id1"], irrelevant_ids=["id2"], stats_path=path)
  stats = load_stats(path)
  assert stats["id1"]["usage_count"] == 1
  assert stats["id2"]["usage_count"] == 0
