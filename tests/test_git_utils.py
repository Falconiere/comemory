import subprocess
from pathlib import Path

import pytest

from qwick_memory.git_utils import detect_author, detect_repo_name, git_sync


def test_detect_repo_name_from_remote(tmp_path: Path):
  """Extracts repo name from git remote URL."""
  subprocess.run(["git", "init"], cwd=tmp_path, capture_output=True)
  subprocess.run(
    ["git", "remote", "add", "origin", "https://github.com/SidegigLLC/qwick-backend.git"],
    cwd=tmp_path,
    capture_output=True,
  )
  name = detect_repo_name(tmp_path)
  assert name == "qwick-backend"


def test_detect_repo_name_ssh_remote(tmp_path: Path):
  """Extracts repo name from SSH remote URL."""
  subprocess.run(["git", "init"], cwd=tmp_path, capture_output=True)
  subprocess.run(
    ["git", "remote", "add", "origin", "git@github.com:SidegigLLC/qwick-backend.git"],
    cwd=tmp_path,
    capture_output=True,
  )
  name = detect_repo_name(tmp_path)
  assert name == "qwick-backend"


def test_detect_repo_name_returns_none_when_no_remote(tmp_path: Path):
  """Returns None when git repo has no remote."""
  subprocess.run(["git", "init"], cwd=tmp_path, capture_output=True)
  assert detect_repo_name(tmp_path) is None


def test_detect_repo_name_returns_none_when_no_git(tmp_path: Path):
  """Returns None when not in a git repo."""
  assert detect_repo_name(tmp_path) is None


def test_detect_repo_name_uses_claude_project_dir(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
  """Uses CLAUDE_PROJECT_DIR env var when cwd is not passed (MCP server context)."""
  project = tmp_path / "my-project"
  project.mkdir()
  subprocess.run(["git", "init"], cwd=project, capture_output=True)
  subprocess.run(
    ["git", "remote", "add", "origin", "https://github.com/org/my-project.git"],
    cwd=project,
    capture_output=True,
  )
  monkeypatch.setenv("CLAUDE_PROJECT_DIR", str(project))
  name = detect_repo_name()  # no cwd — should use CLAUDE_PROJECT_DIR
  assert name == "my-project"


def test_detect_repo_name_explicit_cwd_overrides_claude_project_dir(
  tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
  """Explicit cwd parameter takes precedence over CLAUDE_PROJECT_DIR."""
  project = tmp_path / "explicit-repo"
  project.mkdir()
  subprocess.run(["git", "init"], cwd=project, capture_output=True)
  subprocess.run(
    ["git", "remote", "add", "origin", "https://github.com/org/explicit-repo.git"],
    cwd=project,
    capture_output=True,
  )
  monkeypatch.setenv("CLAUDE_PROJECT_DIR", "/some/other/dir")
  name = detect_repo_name(project)
  assert name == "explicit-repo"


def test_detect_author(tmp_path: Path):
  """Reads author from git config."""
  subprocess.run(["git", "init"], cwd=tmp_path, capture_output=True)
  subprocess.run(
    ["git", "config", "user.name", "Test User"],
    cwd=tmp_path,
    capture_output=True,
  )
  author = detect_author(tmp_path)
  assert author == "Test User"


def test_detect_author_falls_back_to_unknown(tmp_path: Path, monkeypatch):
  """Returns 'unknown' when git user.name is not set."""
  monkeypatch.setenv("GIT_CONFIG_GLOBAL", "/dev/null")
  monkeypatch.setenv("GIT_CONFIG_SYSTEM", "/dev/null")
  subprocess.run(["git", "init"], cwd=tmp_path, capture_output=True)
  author = detect_author(tmp_path)
  assert author == "unknown"


def test_git_sync_creates_orphan_branch_and_commits(tmp_path: Path):
  """git_sync initialises an orphan 'memories' branch and commits files."""
  memories = tmp_path / "memories"
  memories.mkdir(parents=True)
  (memories / "abc123.md").write_text("test content")

  git_sync(tmp_path, "save: abc123")

  # Verify git repo was created on the 'memories' branch
  branch = subprocess.run(
    ["git", "branch", "--show-current"],
    cwd=tmp_path,
    capture_output=True,
    text=True,
  )
  assert branch.stdout.strip() == "memories"

  # Verify commit exists
  log = subprocess.run(
    ["git", "log", "--oneline"],
    cwd=tmp_path,
    capture_output=True,
    text=True,
  )
  assert log.returncode == 0
  assert "save: abc123" in log.stdout


def test_git_sync_creates_gitignore(tmp_path: Path):
  """git_sync creates a .gitignore that excludes .vectordb/."""
  memories = tmp_path / "memories"
  memories.mkdir(parents=True)
  (memories / "abc123.md").write_text("test content")

  git_sync(tmp_path, "save: abc123")

  gitignore = tmp_path / ".gitignore"
  assert gitignore.exists()
  assert ".vectordb/" in gitignore.read_text()


def test_git_sync_no_commit_when_nothing_staged(tmp_path: Path):
  """git_sync does not create empty commits."""
  # Pre-init so _ensure_rag_repo sees it as ready
  subprocess.run(["git", "init"], cwd=tmp_path, capture_output=True)
  subprocess.run(["git", "checkout", "--orphan", "memories"], cwd=tmp_path, capture_output=True)

  git_sync(tmp_path, "should not appear")

  result = subprocess.run(
    ["git", "log", "--oneline"],
    cwd=tmp_path,
    capture_output=True,
    text=True,
  )
  # No commits should exist
  assert result.returncode != 0 or "should not appear" not in result.stdout


def test_git_sync_never_raises(tmp_path: Path):
  """git_sync is best-effort and never raises even on failure."""
  # Pass a non-existent deep path that can't be created
  git_sync(Path("/dev/null/impossible"), "fail gracefully")


def test_git_sync_skips_setup_when_already_ready(tmp_path: Path):
  """git_sync skips _ensure_rag_repo on subsequent calls (cached)."""
  memories = tmp_path / "memories"
  memories.mkdir(parents=True)
  (memories / "first.md").write_text("first")

  git_sync(tmp_path, "first save")

  # Second call should reuse the cached state
  (memories / "second.md").write_text("second")
  git_sync(tmp_path, "second save")

  log = subprocess.run(
    ["git", "log", "--oneline"],
    cwd=tmp_path,
    capture_output=True,
    text=True,
  )
  assert "first save" in log.stdout
  assert "second save" in log.stdout
