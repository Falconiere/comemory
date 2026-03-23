"""Auto-detect repo name and author from git context, and sync memories via git."""

from __future__ import annotations

import logging
import os
import subprocess
from pathlib import Path

logger = logging.getLogger(__name__)

MEMORIES_BRANCH = "memories"

_rag_repo_ready: Path | None = None  # caches the rag_dir that was set up


def _run_git(args: list[str], cwd: Path) -> subprocess.CompletedProcess[str]:
  """Run a git command, returning the CompletedProcess."""
  return subprocess.run(
    ["git", *args],
    cwd=cwd,
    capture_output=True,
    text=True,
    timeout=30,
  )


def _is_git_repo(path: Path) -> bool:
  """Check if path is inside a git repository."""
  try:
    result = _run_git(["rev-parse", "--git-dir"], cwd=path)
    return result.returncode == 0
  except (subprocess.TimeoutExpired, FileNotFoundError):
    return False


def _has_remote(cwd: Path) -> bool:
  """Check if the git repo has at least one remote configured."""
  try:
    result = _run_git(["remote"], cwd=cwd)
    return result.returncode == 0 and bool(result.stdout.strip())
  except (subprocess.TimeoutExpired, FileNotFoundError):
    return False



def _ensure_rag_repo(rag_dir: Path) -> None:
  """Ensure rag_dir is a git repo on the orphan 'memories' branch.

  On first call:
    1. git init + checkout --orphan memories
    2. Write .gitignore for .vectordb/
    3. If QWICK_MEMORY_REMOTE is set, configure remote and pull existing memories
    4. Otherwise, create an initial commit (local-only)
  """
  global _rag_repo_ready
  if _rag_repo_ready == rag_dir:
    return

  rag_dir.mkdir(parents=True, exist_ok=True)

  if _is_git_repo(rag_dir):
    _rag_repo_ready = rag_dir
    return

  # --- First-time setup ---
  _run_git(["init"], cwd=rag_dir)
  _run_git(["checkout", "--orphan", MEMORIES_BRANCH], cwd=rag_dir)
  logger.info("Initialised memories repo at %s", rag_dir)

  # .gitignore — keep .vectordb/ out of version control
  gitignore = rag_dir / ".gitignore"
  if not gitignore.exists():
    gitignore.write_text(".vectordb/\n")

  # Configure remote only when explicitly set via QWICK_MEMORY_REMOTE.
  # Previously auto-detected from the plugin source repo, which caused memories
  # to be pushed to the qwick-memory GitHub repo instead of a user-controlled repo.
  remote_url = os.environ.get("QWICK_MEMORY_REMOTE") or None

  if remote_url:
    _run_git(["remote", "add", "origin", remote_url], cwd=rag_dir)
    logger.info("Configured remote: %s", remote_url)

    # Pull existing memories from remote if the branch exists
    fetch = _run_git(["fetch", "origin", MEMORIES_BRANCH], cwd=rag_dir)
    if fetch.returncode == 0:
      _run_git(["reset", "--hard", "origin/" + MEMORIES_BRANCH], cwd=rag_dir)
      logger.info("Pulled existing memories from remote.")
      _rag_repo_ready = rag_dir
      return

  # No remote or no existing branch — seed with .gitignore commit
  _run_git(["add", ".gitignore"], cwd=rag_dir)
  _run_git(["commit", "-m", "init memories branch"], cwd=rag_dir)
  _rag_repo_ready = rag_dir


def git_sync(rag_dir: Path, message: str) -> None:
  """Auto-commit and push memory changes. Best-effort — never raises.

  1. Ensure rag_dir is set up (orphan branch, remote).
  2. Stage all changes under memories/.
  3. Commit with the given message.
  4. Pull --rebase then push to origin/memories.
  """
  try:
    _ensure_rag_repo(rag_dir)

    # Stage memory files + .gitignore
    _run_git(["add", "memories/"], cwd=rag_dir)
    gitignore = rag_dir / ".gitignore"
    if gitignore.exists():
      _run_git(["add", ".gitignore"], cwd=rag_dir)

    # Check if there's anything to commit
    status = _run_git(["diff", "--cached", "--quiet"], cwd=rag_dir)
    if status.returncode == 0:
      return

    _run_git(["commit", "-m", message], cwd=rag_dir)
    logger.info("Committed: %s", message)

    if _has_remote(rag_dir):
      # Rebase on remote changes before pushing (handles multi-machine use)
      rebase = _run_git(["pull", "--rebase", "origin", MEMORIES_BRANCH], cwd=rag_dir)
      if rebase.returncode != 0:
        _run_git(["rebase", "--abort"], cwd=rag_dir)
        logger.warning("git pull --rebase failed, skipping push: %s", rebase.stderr.strip())
        return
      push = _run_git(["push", "-u", "origin", MEMORIES_BRANCH], cwd=rag_dir)
      if push.returncode == 0:
        logger.info("Pushed to origin/%s.", MEMORIES_BRANCH)
      else:
        logger.warning("git push failed: %s", push.stderr.strip())
  except Exception:
    logger.debug("git_sync failed (best-effort)", exc_info=True)


def detect_repo_name(cwd: Path | None = None) -> str:
  """Detect repository name from git remote URL, falling back to directory name.

  When running as an MCP server plugin, Path.cwd() points to the plugin cache
  directory (e.g. ~/.claude/plugins/cache/.../0.1.0/), not the user's project.
  CLAUDE_PROJECT_DIR is set by Claude Code and points to the actual project.
  """
  if cwd is None:
    project_dir = os.environ.get("CLAUDE_PROJECT_DIR")
    cwd = Path(project_dir) if project_dir else Path.cwd()
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
