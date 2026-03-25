"""Tests for scoring functions — thresholds, freshness decay, combined score."""

from datetime import datetime, timedelta, timezone

from qwick_memory.search import (
  SearchResult,
  _apply_thresholds,
  _compute_final_score,
  _freshness_decay,
)


def _make_result(score: float, reranker_score: float = 0.0, **kwargs) -> SearchResult:
  defaults = dict(
    id="x", repo="r", type="note", tags="", author="a",
    created="2026-01-01T00:00:00", content="c", quality=3,
  )
  defaults.update(kwargs)
  return SearchResult(score=score, reranker_score=reranker_score, **defaults)


# -- _apply_thresholds --

def test_apply_thresholds_filters_below_floor():
  results = [_make_result(0.0, 0.8), _make_result(0.0, 0.2), _make_result(0.0, 0.1)]
  filtered = _apply_thresholds(results, min_score=0.3, max_gap=0.15)
  assert len(filtered) == 1
  assert filtered[0].reranker_score == 0.8


def test_apply_thresholds_gap_detection():
  results = [_make_result(0.0, 0.82), _make_result(0.0, 0.79), _make_result(0.0, 0.41), _make_result(0.0, 0.38)]
  filtered = _apply_thresholds(results, min_score=0.3, max_gap=0.15)
  assert len(filtered) == 2


def test_apply_thresholds_empty_list():
  assert _apply_thresholds([], min_score=0.3, max_gap=0.15) == []


def test_apply_thresholds_single_above():
  results = [_make_result(0.0, 0.5)]
  assert len(_apply_thresholds(results, min_score=0.3, max_gap=0.15)) == 1


def test_apply_thresholds_all_below():
  results = [_make_result(0.0, 0.1), _make_result(0.0, 0.05)]
  assert _apply_thresholds(results, min_score=0.3, max_gap=0.15) == []


# -- _freshness_decay --

def test_freshness_decay_convention_365_half_life():
  created = datetime.now(timezone.utc) - timedelta(days=365)
  decay = _freshness_decay(created, "convention")
  assert 0.45 < decay < 0.55


def test_freshness_decay_session_summary_14_half_life():
  created = datetime.now(timezone.utc) - timedelta(days=14)
  decay = _freshness_decay(created, "session-summary")
  assert 0.45 < decay < 0.55


def test_freshness_decay_brand_new():
  created = datetime.now(timezone.utc)
  decay = _freshness_decay(created, "note")
  assert decay > 0.99


def test_freshness_decay_unknown_type_uses_default():
  created = datetime.now(timezone.utc) - timedelta(days=90)
  decay = _freshness_decay(created, "unknown_type")
  assert 0.45 < decay < 0.55


# -- _compute_final_score --

def test_compute_final_score_all_perfect():
  score = _compute_final_score(
    reranker_score=0.9,
    memory_type="convention",
    created=datetime.now(timezone.utc),
    quality=5,
    stats=None,
  )
  # freshness ~1.0, quality_boost = 1.0, usage_boost = 0.9
  assert 0.8 < score < 0.95


def test_compute_final_score_low_quality():
  score = _compute_final_score(
    reranker_score=0.9,
    memory_type="convention",
    created=datetime.now(timezone.utc),
    quality=1,
    stats=None,
  )
  # quality_boost = 0.68, usage_boost = 0.9
  assert 0.5 < score < 0.7
