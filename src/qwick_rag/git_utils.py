"""Auto-detect repo name and author from git context."""

import logging
import subprocess
from pathlib import Path

logger = logging.getLogger(__name__)


def detect_repo_name(cwd: Path | None = None) -> str:
  """Detect repository name from git remote URL, falling back to directory name."""
  cwd = cwd or Path.cwd()
  try:
    result = subprocess.run(
      ["git", "remote", "get-url", "origin"],
      cwd=cwd,
      capture_output=True,
      text=True,
      timeout=5,
    )
    if result.returncode == 0 and result.stdout.strip():
      url = result.stdout.strip()
      name = url.rstrip("/").split("/")[-1]
      if name.endswith(".git"):
        name = name[:-4]
      return name
  except (subprocess.TimeoutExpired, FileNotFoundError):
    pass

  logger.warning("No git remote found, using directory name as repo: %s", cwd.name)
  return cwd.name


def detect_author(cwd: Path | None = None) -> str:
  """Detect author from git config user.name, falling back to 'unknown'."""
  cwd = cwd or Path.cwd()
  try:
    result = subprocess.run(
      ["git", "config", "user.name"],
      cwd=cwd,
      capture_output=True,
      text=True,
      timeout=5,
    )
    if result.returncode == 0 and result.stdout.strip():
      return result.stdout.strip()
  except (subprocess.TimeoutExpired, FileNotFoundError):
    pass

  logger.warning("Could not detect git user.name, using 'unknown'")
  return "unknown"
