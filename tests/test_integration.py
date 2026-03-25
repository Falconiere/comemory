"""End-to-end integration test: save → search → list → delete → index → doctor."""

from pathlib import Path

from typer.testing import CliRunner

from qwick_memory.cli import app

runner = CliRunner()


def test_full_lifecycle(tmp_path: Path, monkeypatch):
  """Test the complete memory lifecycle."""
  rag_dir = tmp_path / "rag"
  rag_dir.mkdir()
  (rag_dir / "memories").mkdir()
  monkeypatch.setenv("QWICK_MEMORY_DIR", str(rag_dir))
  monkeypatch.setenv("QWICK_MEMORY_REPO", "integration-test")
  monkeypatch.setenv("QWICK_MEMORY_AUTHOR", "tester")

  # Save three memories
  result = runner.invoke(
    app,
    ["save", "PostgreSQL is our primary database", "--type", "decision", "--tags", "db,postgres"],
  )
  assert result.exit_code == 0, result.output

  result = runner.invoke(
    app, ["save", "Redis handles session caching", "--type", "decision", "--tags", "cache,redis"]
  )
  assert result.exit_code == 0, result.output

  result = runner.invoke(
    app, ["save", "Always use named exports in React", "--type", "convention", "--tags", "react"]
  )
  assert result.exit_code == 0, result.output

  # List should show 3
  result = runner.invoke(app, ["list"])
  assert result.exit_code == 0
  assert "3 memories found." in result.output

  # Search should find postgres memory
  result = runner.invoke(app, ["search", "which database do we use"])
  assert result.exit_code == 0
  assert "PostgreSQL" in result.output

  # Search with type filter
  result = runner.invoke(app, ["search", "React export convention", "--type", "convention"])
  assert result.exit_code == 0
  # Content is "Always use named exports in React"; table may wrap it across lines
  assert "convention" in result.output
  assert "React" in result.output

  # Delete one memory
  files = list((rag_dir / "memories").glob("*.md"))
  first_id = files[0].stem
  result = runner.invoke(app, ["delete", first_id])
  assert result.exit_code == 0

  # List should show 2
  result = runner.invoke(app, ["list"])
  assert "2 memories found." in result.output

  # Rebuild index
  result = runner.invoke(app, ["index"])
  assert result.exit_code == 0

  # Doctor should pass
  result = runner.invoke(app, ["doctor"])
  assert result.exit_code == 0
