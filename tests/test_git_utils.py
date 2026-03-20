import subprocess
from pathlib import Path

from qwick_rag.git_utils import detect_author, detect_repo_name


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


def test_detect_repo_name_falls_back_to_dirname(tmp_path: Path):
  """Falls back to directory name when no remote exists."""
  subprocess.run(["git", "init"], cwd=tmp_path, capture_output=True)
  name = detect_repo_name(tmp_path)
  assert name == tmp_path.name


def test_detect_repo_name_no_git(tmp_path: Path):
  """Falls back to directory name when not in a git repo."""
  name = detect_repo_name(tmp_path)
  assert name == tmp_path.name


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
