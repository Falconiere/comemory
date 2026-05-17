"""Tests for qwick_memory.config — env-resolved tunable constants."""

import importlib

import pytest


def _reload_config():
  """Re-import config so module-level env reads happen again."""
  import qwick_memory.config as cfg

  return importlib.reload(cfg)


def test_min_relevance_default(monkeypatch: pytest.MonkeyPatch) -> None:
  monkeypatch.delenv("QWICK_MEMORY_MIN_RELEVANCE", raising=False)
  cfg = _reload_config()
  assert cfg.MIN_RELEVANCE_SCORE == 0.3


def test_min_relevance_env_override(monkeypatch: pytest.MonkeyPatch) -> None:
  monkeypatch.setenv("QWICK_MEMORY_MIN_RELEVANCE", "0.55")
  cfg = _reload_config()
  assert cfg.MIN_RELEVANCE_SCORE == 0.55


def test_max_gap_env_override(monkeypatch: pytest.MonkeyPatch) -> None:
  monkeypatch.setenv("QWICK_MEMORY_MAX_GAP", "0.25")
  cfg = _reload_config()
  assert cfg.MAX_SCORE_GAP == 0.25


def test_hybrid_weight_default(monkeypatch: pytest.MonkeyPatch) -> None:
  monkeypatch.delenv("QWICK_MEMORY_HYBRID_WEIGHT", raising=False)
  cfg = _reload_config()
  assert cfg.HYBRID_WEIGHT == 0.5


def test_reranker_model_env_override(monkeypatch: pytest.MonkeyPatch) -> None:
  monkeypatch.setenv("QWICK_MEMORY_RERANKER_MODEL", "custom/model")
  cfg = _reload_config()
  assert cfg.RERANKER_MODEL == "custom/model"
