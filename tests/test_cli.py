"""Tests for qwick_rag.cli — Typer CLI commands."""

from pathlib import Path

import pytest
from typer.testing import CliRunner

from qwick_rag.cli import app

runner = CliRunner()


@pytest.fixture(autouse=True)
def _cli_env(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
  """Set environment variables so CLI uses temp directories."""
  monkeypatch.setenv("QWICK_RAG_DIR", str(tmp_path))
  monkeypatch.setenv("QWICK_RAG_REPO", "test-repo")
  monkeypatch.setenv("QWICK_RAG_AUTHOR", "tester")
  # Create memories directory
  (tmp_path / "memories").mkdir()


def test_save_creates_memory_file(tmp_path: Path) -> None:
  """save creates a .md file in the memories directory."""
  result = runner.invoke(app, ["save", "Remember to test the CLI"])
  assert result.exit_code == 0, result.output
  md_files = list((tmp_path / "memories").rglob("*.md"))
  assert len(md_files) == 1


def test_search_returns_results(tmp_path: Path) -> None:
  """save then search finds the saved memory."""
  result = runner.invoke(app, ["save", "PostgreSQL is great for JSONB"])
  assert result.exit_code == 0, result.output

  result = runner.invoke(app, ["search", "PostgreSQL"])
  assert result.exit_code == 0, result.output
  assert "PostgreSQL" in result.output


def test_list_shows_memories(tmp_path: Path) -> None:
  """save then list shows the memory."""
  result = runner.invoke(app, ["save", "List test memory content"])
  assert result.exit_code == 0, result.output

  result = runner.invoke(app, ["list"])
  assert result.exit_code == 0, result.output
  assert "List test memory" in result.output


def test_delete_removes_memory(tmp_path: Path) -> None:
  """save then delete removes the file."""
  result = runner.invoke(app, ["save", "Delete me later"])
  assert result.exit_code == 0, result.output

  md_files = list((tmp_path / "memories").rglob("*.md"))
  assert len(md_files) == 1

  # Extract memory ID from filename
  memory_id = md_files[0].stem
  result = runner.invoke(app, ["delete", memory_id])
  assert result.exit_code == 0, result.output

  md_files = list((tmp_path / "memories").rglob("*.md"))
  assert len(md_files) == 0


def test_index_command(tmp_path: Path) -> None:
  """index runs successfully after saving a memory."""
  result = runner.invoke(app, ["save", "Indexing test content"])
  assert result.exit_code == 0, result.output

  result = runner.invoke(app, ["index"])
  assert result.exit_code == 0, result.output
  assert "Index built" in result.output


def test_context_shows_memories(tmp_path: Path) -> None:
  """context command shows recent memories."""
  result = runner.invoke(app, ["save", "Context test memory content"])
  assert result.exit_code == 0, result.output

  result = runner.invoke(app, ["context"])
  assert result.exit_code == 0, result.output
  assert "Recent Memories" in result.output
  assert "Context test memory" in result.output


def test_context_empty(tmp_path: Path) -> None:
  """context command on empty repo shows 'No memories found'."""
  result = runner.invoke(app, ["context"])
  assert result.exit_code == 0, result.output
  assert "No memories found" in result.output


def test_context_limit(tmp_path: Path) -> None:
  """context --limit restricts number of memories shown."""
  for i in range(5):
    result = runner.invoke(app, ["save", f"Memory number {i} unique content here"])
    assert result.exit_code == 0, result.output

  result = runner.invoke(app, ["context", "--limit", "2"])
  assert result.exit_code == 0, result.output
  memory_lines = [ln for ln in result.output.splitlines() if ln.startswith("- [")]
  assert len(memory_lines) == 2, f"Expected 2 memories, got {len(memory_lines)}"
