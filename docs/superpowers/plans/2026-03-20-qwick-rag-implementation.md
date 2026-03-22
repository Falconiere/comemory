# qwick-memory Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Python CLI + Claude Code MCP plugin that stores and retrieves memories across multiple repositories using markdown files as source of truth and LanceDB for vector search.

**Architecture:** Memories are markdown files with YAML frontmatter stored in `memories/{repo}/{id}.md`. LanceDB provides embedded vector search (gitignored, rebuilt from files). fastembed handles local ONNX-based embeddings. The tool exposes both a Typer CLI and an MCP server for Claude Code integration.

**Tech Stack:** Python 3.10+, LanceDB, fastembed, python-frontmatter, Typer, Rich, MCP Python SDK (FastMCP)

**Spec:** `docs/superpowers/specs/2026-03-20-qwick-memory-design.md`

---

## File Map

| File | Responsibility |
|------|---------------|
| `pyproject.toml` | Package config, dependencies, CLI entry point |
| `src/qwick_rag/__init__.py` | Package init, version |
| `src/qwick_rag/__main__.py` | `python -m qwick_rag` entry point |
| `src/qwick_rag/errors.py` | Structured error types (QwickRagError hierarchy) |
| `src/qwick_rag/config.py` | Shared helpers: resolve paths, get repo/author/index (used by CLI + MCP) |
| `src/qwick_rag/git_utils.py` | Auto-detect repo name and author from git context |
| `src/qwick_rag/memory.py` | Memory dataclass, parse/write markdown with frontmatter, ID generation |
| `src/qwick_rag/index.py` | LanceDB connection, embed, upsert, delete, rebuild, FTS index, optimize |
| `src/qwick_rag/search.py` | Hybrid search (vector + BM25 + RRF), metadata filtering, result formatting |
| `src/qwick_rag/cli.py` | Typer CLI commands (save, search, list, delete, index, doctor) |
| `src/qwick_rag/server.py` | MCP server (FastMCP) exposing rag_save, rag_search, rag_list, rag_delete, rag_index, rag_context |
| `tests/conftest.py` | Shared fixtures (tmp memories dir, tmp vectordb, sample memories) |
| `tests/test_git_utils.py` | Tests for repo/author detection |
| `tests/test_memory.py` | Tests for memory model, frontmatter parsing, ID generation |
| `tests/test_index.py` | Tests for LanceDB indexing, incremental rebuild, orphan cleanup |
| `tests/test_search.py` | Tests for hybrid search, filtering, result ranking |
| `tests/test_cli.py` | Integration tests for CLI commands via Typer CliRunner |
| `tests/test_server.py` | Tests for MCP server tool invocations |
| `.claude-plugin/plugin.json` | Claude Code plugin manifest |
| `.claude-plugin/marketplace.json` | Marketplace distribution metadata |
| `.mcp.json` | MCP server launch config |
| `hooks/hooks.json` | SessionStart hook config |
| `scripts/session-start.sh` | Auto-index on Claude Code session start |
| `skills/memory/SKILL.md` | Memory protocol instructions for Claude |

---

### Task 1: Project Scaffolding

**Files:**
- Create: `pyproject.toml`
- Create: `src/qwick_rag/__init__.py`
- Create: `src/qwick_rag/__main__.py`
- Modify: `.gitignore`

- [ ] **Step 1: Create pyproject.toml**

```toml
[build-system]
requires = ["hatchling"]
build-backend = "hatchling.build"

[project]
name = "qwick-memory"
version = "0.1.0"
description = "Centralized RAG memory for multiple repositories"
readme = "README.md"
requires-python = ">=3.10"
license = "MIT"
dependencies = [
    "lancedb>=0.30",
    "fastembed>=0.7",
    "python-frontmatter>=1.1",
    "typer[all]>=0.12",
    "rich>=13",
    "mcp>=1.2",
]

[project.scripts]
qwick-memory = "qwick_rag.cli:app"

[project.optional-dependencies]
dev = [
    "pytest>=8",
    "pytest-asyncio>=0.24",
    "ruff>=0.9",
    "pyright>=1.1",
]

[tool.hatch.build.targets.wheel]
packages = ["src/qwick_rag"]

[tool.pytest.ini_options]
testpaths = ["tests"]
pythonpath = ["src"]

[tool.ruff]
indent-width = 2
line-length = 100
src = ["src", "tests"]

[tool.ruff.lint]
select = [
    "E",    # pycodestyle errors
    "W",    # pycodestyle warnings
    "F",    # pyflakes
    "I",    # isort
    "N",    # pep8-naming
    "UP",   # pyupgrade
    "B",    # flake8-bugbear
    "SIM",  # flake8-simplify
    "TCH",  # flake8-type-checking
    "RUF",  # ruff-specific rules
]

[tool.ruff.format]
indent-style = "space"
quote-style = "double"

[tool.pyright]
pythonVersion = "3.10"
pythonPlatform = "All"
typeCheckingMode = "standard"
venvPath = "."
venv = ".venv"
```

- [ ] **Step 2: Create package init**

```python
# src/qwick_rag/__init__.py
"""qwick-memory: Centralized RAG memory for multiple repositories."""

__version__ = "0.1.0"
```

- [ ] **Step 3: Create __main__.py**

```python
# src/qwick_rag/__main__.py
"""Allow running as python -m qwick_rag."""

from qwick_rag.cli import app

app()
```

- [ ] **Step 4: Update .gitignore to add .vectordb/ and memories/**

Append to existing `.gitignore`:

```
# qwick-memory
.vectordb/
```

Note: `memories/` is NOT gitignored — it's the shared source of truth.

- [ ] **Step 5: Create directories**

```bash
mkdir -p src/qwick_rag tests memories
```

- [ ] **Step 6: Install in dev mode and verify**

```bash
cd /Users/falconiere/Projects/qwick-memory
uv venv && source .venv/bin/activate.fish
uv pip install -e ".[dev]"
```

Verify: `python -c "import qwick_rag; print(qwick_rag.__version__)"` → `0.1.0`

- [ ] **Step 7: Verify quality tools work**

```bash
ruff check src/ tests/
ruff format --check src/ tests/
pyright src/
```

Expected: All pass (no files to check yet, but tools should run without error).

- [ ] **Step 8: Commit**

```bash
git add pyproject.toml src/ .gitignore
git commit -m "feat: scaffold Python package with dependencies and quality tools"
```

---

### Task 2: Error Types

**Files:**
- Create: `src/qwick_rag/errors.py`
- Create: `tests/test_errors.py`

- [ ] **Step 1: Write the failing test**

```python
# tests/test_errors.py
from qwick_rag.errors import (
    QwickRagError,
    StorageError,
    VectorIndexError,
    GitError,
    MemoryParseError,
    ConfigError,
)


def test_error_hierarchy():
    """All custom errors inherit from QwickRagError."""
    assert issubclass(StorageError, QwickRagError)
    assert issubclass(VectorIndexError, QwickRagError)
    assert issubclass(GitError, QwickRagError)
    assert issubclass(MemoryParseError, QwickRagError)
    assert issubclass(ConfigError, QwickRagError)


def test_error_carries_context():
    """Errors carry message, suggested_fix, and context."""
    err = StorageError(
        "Cannot write file",
        suggested_fix="Check disk space",
        context={"path": "/tmp/foo.md"},
    )
    assert str(err) == "Cannot write file"
    assert err.suggested_fix == "Check disk space"
    assert err.context["path"] == "/tmp/foo.md"
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pytest tests/test_errors.py -v`
Expected: FAIL — `ModuleNotFoundError: No module named 'qwick_rag.errors'`

- [ ] **Step 3: Implement errors.py**

```python
# src/qwick_rag/errors.py
"""Structured error types for qwick-memory."""

from typing import Any


class QwickRagError(Exception):
    """Base error for all qwick-memory errors."""

    def __init__(
        self,
        message: str,
        suggested_fix: str = "",
        context: dict[str, Any] | None = None,
    ):
        super().__init__(message)
        self.suggested_fix = suggested_fix
        self.context = context or {}


class StorageError(QwickRagError):
    """File system issues (permissions, disk full, path not found)."""


class VectorIndexError(QwickRagError):
    """LanceDB issues (corrupt DB, embedding failures)."""


class GitError(QwickRagError):
    """Git detection failures (no remote, no user config)."""


class MemoryParseError(QwickRagError):
    """Malformed frontmatter, invalid YAML, missing required fields."""


class ConfigError(QwickRagError):
    """Invalid config, missing dependencies."""
```

- [ ] **Step 4: Run test to verify it passes**

Run: `pytest tests/test_errors.py -v`
Expected: 2 passed

- [ ] **Step 5: Commit**

```bash
git add src/qwick_rag/errors.py tests/test_errors.py
git commit -m "feat: add structured error types"
```

---

### Task 3: Git Utilities

**Files:**
- Create: `src/qwick_rag/git_utils.py`
- Create: `tests/test_git_utils.py`

- [ ] **Step 1: Write the failing tests**

```python
# tests/test_git_utils.py
import subprocess
from pathlib import Path

from qwick_rag.git_utils import detect_repo_name, detect_author


def test_detect_repo_name_from_remote(tmp_path: Path):
    """Extracts repo name from git remote URL."""
    # Set up a git repo with a remote
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
    # Isolate from global/system git config
    monkeypatch.setenv("GIT_CONFIG_GLOBAL", "/dev/null")
    monkeypatch.setenv("GIT_CONFIG_SYSTEM", "/dev/null")
    subprocess.run(["git", "init"], cwd=tmp_path, capture_output=True)
    author = detect_author(tmp_path)
    assert author == "unknown"
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `pytest tests/test_git_utils.py -v`
Expected: FAIL — `ModuleNotFoundError`

- [ ] **Step 3: Implement git_utils.py**

```python
# src/qwick_rag/git_utils.py
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
            # Handle both HTTPS and SSH URLs
            # https://github.com/org/repo.git -> repo
            # git@github.com:org/repo.git -> repo
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `pytest tests/test_git_utils.py -v`
Expected: 6 passed

- [ ] **Step 5: Commit**

```bash
git add src/qwick_rag/git_utils.py tests/test_git_utils.py
git commit -m "feat: add git repo and author auto-detection"
```

---

### Task 3b: Shared Config Module

**Files:**
- Create: `src/qwick_rag/config.py`

- [ ] **Step 1: Create config.py**

```python
# src/qwick_rag/config.py
"""Shared helpers: resolve paths, get repo/author/index. Used by CLI + MCP server."""

import os
from pathlib import Path

from qwick_rag.git_utils import detect_author, detect_repo_name


def get_rag_dir() -> Path:
    """Resolve the qwick-memory root directory."""
    env = os.environ.get("QWICK_MEMORY_DIR")
    if env:
        return Path(env)
    return Path.cwd()


def get_memories_dir() -> Path:
    return get_rag_dir() / "memories"


def get_vectordb_dir() -> Path:
    return get_rag_dir() / ".vectordb"


def get_repo() -> str:
    env = os.environ.get("QWICK_MEMORY_REPO")
    if env:
        return env
    return detect_repo_name()


def get_author() -> str:
    env = os.environ.get("QWICK_MEMORY_AUTHOR")
    if env:
        return env
    return detect_author()


def get_index():
    """Lazy import to avoid circular dependency."""
    from qwick_rag.index import MemoryIndex
    return MemoryIndex(vectordb_dir=get_vectordb_dir())
```

- [ ] **Step 2: Commit**

```bash
git add src/qwick_rag/config.py
git commit -m "feat: add shared config module for path and context helpers"
```

---

### Task 4: Memory Model

**Files:**
- Create: `src/qwick_rag/memory.py`
- Create: `tests/test_memory.py`

- [ ] **Step 1: Write the failing tests**

```python
# tests/test_memory.py
from datetime import datetime, timezone
from pathlib import Path

from qwick_rag.memory import Memory, generate_id, parse_memory, write_memory

SAMPLE_CONTENT = "We chose PostgreSQL over MongoDB for strong transactional guarantees."


def test_generate_id_is_deterministic():
    """Same content always produces the same ID."""
    id1 = generate_id(SAMPLE_CONTENT)
    id2 = generate_id(SAMPLE_CONTENT)
    assert id1 == id2


def test_generate_id_is_12_hex_chars():
    """ID is 12 hex characters (48 bits)."""
    id_ = generate_id(SAMPLE_CONTENT)
    assert len(id_) == 12
    assert all(c in "0123456789abcdef" for c in id_)


def test_generate_id_differs_for_different_content():
    """Different content produces different IDs."""
    id1 = generate_id("foo")
    id2 = generate_id("bar")
    assert id1 != id2


def test_memory_dataclass():
    """Memory holds all required fields."""
    mem = Memory(
        id="abc123def456",
        repo="qwick-backend",
        type="decision",
        tags=["database", "postgres"],
        author="falconiere",
        created=datetime(2026, 3, 20, 14, 30, tzinfo=timezone.utc),
        content=SAMPLE_CONTENT,
    )
    assert mem.id == "abc123def456"
    assert mem.repo == "qwick-backend"
    assert mem.type == "decision"
    assert mem.tags == ["database", "postgres"]


def test_write_and_parse_roundtrip(tmp_path: Path):
    """Writing a memory and parsing it back produces the same data."""
    mem = Memory(
        id="abc123def456",
        repo="qwick-backend",
        type="decision",
        tags=["database"],
        author="falconiere",
        created=datetime(2026, 3, 20, 14, 30, tzinfo=timezone.utc),
        content=SAMPLE_CONTENT,
    )
    filepath = tmp_path / "abc123def456.md"
    write_memory(mem, filepath)

    parsed = parse_memory(filepath)
    assert parsed.id == mem.id
    assert parsed.repo == mem.repo
    assert parsed.type == mem.type
    assert parsed.tags == mem.tags
    assert parsed.author == mem.author
    assert parsed.content == mem.content


def test_parse_memory_invalid_yaml(tmp_path: Path):
    """Raises MemoryParseError for malformed frontmatter."""
    filepath = tmp_path / "bad.md"
    filepath.write_text("---\ninvalid: [\n---\ncontent")

    from qwick_rag.errors import MemoryParseError
    import pytest
    with pytest.raises(MemoryParseError):
        parse_memory(filepath)


def test_write_memory_creates_frontmatter(tmp_path: Path):
    """Written file contains valid YAML frontmatter."""
    mem = Memory(
        id="abc123def456",
        repo="test-repo",
        type="note",
        tags=[],
        author="tester",
        created=datetime(2026, 3, 20, tzinfo=timezone.utc),
        content="A simple note.",
    )
    filepath = tmp_path / "test.md"
    write_memory(mem, filepath)
    text = filepath.read_text()
    assert text.startswith("---\n")
    assert "id: abc123def456" in text
    assert "A simple note." in text
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `pytest tests/test_memory.py -v`
Expected: FAIL — `ModuleNotFoundError`

- [ ] **Step 3: Implement memory.py**

```python
# src/qwick_rag/memory.py
"""Memory model: dataclass, parse/write markdown with YAML frontmatter, ID generation."""

import hashlib
import logging
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Literal

import frontmatter

from qwick_rag.errors import MemoryParseError

logger = logging.getLogger(__name__)

MEMORY_TYPES = ("decision", "bug", "convention", "discovery", "pattern", "preference", "note")
MemoryType = Literal["decision", "bug", "convention", "discovery", "pattern", "preference", "note"]


def generate_id(content: str) -> str:
    """Generate a 12-character hex ID from SHA-256 of content."""
    return hashlib.sha256(content.encode("utf-8")).hexdigest()[:12]


@dataclass
class Memory:
    """A single memory observation."""

    id: str
    repo: str
    type: MemoryType
    tags: list[str]
    author: str
    created: datetime
    content: str
    content_hash: str = field(default="", init=False)

    def __post_init__(self):
        self.content_hash = hashlib.sha256(self.content.encode("utf-8")).hexdigest()


def write_memory(memory: Memory, filepath: Path) -> None:
    """Write a memory as a markdown file with YAML frontmatter."""
    post = frontmatter.Post(
        content=memory.content,
        handler=frontmatter.YAMLHandler(),
        id=memory.id,
        repo=memory.repo,
        type=memory.type,
        tags=memory.tags,
        author=memory.author,
        created=memory.created.isoformat(),
    )
    filepath.parent.mkdir(parents=True, exist_ok=True)
    filepath.write_text(frontmatter.dumps(post) + "\n")


def parse_memory(filepath: Path) -> Memory:
    """Parse a markdown file with YAML frontmatter into a Memory."""
    try:
        post = frontmatter.load(str(filepath))
    except Exception as e:
        raise MemoryParseError(
            f"Failed to parse {filepath}: {e}",
            suggested_fix="Check YAML frontmatter syntax",
            context={"path": str(filepath)},
        ) from e

    metadata = post.metadata
    try:
        created_raw = metadata["created"]
        if isinstance(created_raw, datetime):
            created = created_raw
        else:
            created = datetime.fromisoformat(str(created_raw))
        if created.tzinfo is None:
            created = created.replace(tzinfo=timezone.utc)

        return Memory(
            id=str(metadata["id"]),
            repo=str(metadata["repo"]),
            type=metadata["type"],
            tags=metadata.get("tags", []),
            author=str(metadata.get("author", "unknown")),
            created=created,
            content=post.content,
        )
    except KeyError as e:
        raise MemoryParseError(
            f"Missing required field {e} in {filepath}",
            suggested_fix=f"Add {e} to the YAML frontmatter",
            context={"path": str(filepath), "field": str(e)},
        ) from e


def scan_memories(memories_dir: Path) -> list[Path]:
    """Scan memories directory and return all .md file paths."""
    if not memories_dir.exists():
        return []
    return sorted(memories_dir.rglob("*.md"))
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `pytest tests/test_memory.py -v`
Expected: 7 passed

- [ ] **Step 5: Commit**

```bash
git add src/qwick_rag/memory.py tests/test_memory.py
git commit -m "feat: add memory model with frontmatter parsing and ID generation"
```

---

### Task 5: LanceDB Indexing

**Files:**
- Create: `src/qwick_rag/index.py`
- Create: `tests/test_index.py`
- Create: `tests/conftest.py`

- [ ] **Step 1: Create shared test fixtures**

```python
# tests/conftest.py
import pytest
from datetime import datetime, timezone
from pathlib import Path

from qwick_rag.memory import Memory, write_memory


@pytest.fixture
def memories_dir(tmp_path: Path) -> Path:
    """Temporary memories directory with sample memories."""
    d = tmp_path / "memories"
    d.mkdir()
    return d


@pytest.fixture
def vectordb_dir(tmp_path: Path) -> Path:
    """Temporary vectordb directory."""
    d = tmp_path / ".vectordb"
    d.mkdir()
    return d


@pytest.fixture
def sample_memories(memories_dir: Path) -> list[Memory]:
    """Write 3 sample memories and return them."""
    mems = [
        Memory(
            id="aaa111bbb222",
            repo="backend",
            type="decision",
            tags=["database", "postgres"],
            author="alice",
            created=datetime(2026, 3, 20, 10, 0, tzinfo=timezone.utc),
            content="We chose PostgreSQL for the order service due to transactional guarantees.",
        ),
        Memory(
            id="ccc333ddd444",
            repo="backend",
            type="bug",
            tags=["auth", "sessions"],
            author="bob",
            created=datetime(2026, 3, 19, 15, 0, tzinfo=timezone.utc),
            content="Session tokens were not being invalidated on password change. Fixed by adding a token version column.",
        ),
        Memory(
            id="eee555fff666",
            repo="frontend",
            type="convention",
            tags=["react", "components"],
            author="alice",
            created=datetime(2026, 3, 18, 9, 0, tzinfo=timezone.utc),
            content="All React components must use named exports. Default exports are only for pages.",
        ),
    ]
    for mem in mems:
        repo_dir = memories_dir / mem.repo
        repo_dir.mkdir(exist_ok=True)
        write_memory(mem, repo_dir / f"{mem.id}.md")
    return mems
```

- [ ] **Step 2: Write the failing tests**

```python
# tests/test_index.py
from pathlib import Path

from qwick_rag.index import MemoryIndex
from qwick_rag.memory import Memory, write_memory
from datetime import datetime, timezone


def test_build_index_from_memories(memories_dir: Path, vectordb_dir: Path, sample_memories):
    """Building an index from memory files creates LanceDB entries."""
    idx = MemoryIndex(vectordb_dir=vectordb_dir)
    stats = idx.build(memories_dir)
    assert stats["new"] == 3
    assert stats["updated"] == 0
    assert stats["deleted"] == 0


def test_incremental_index_skips_unchanged(memories_dir: Path, vectordb_dir: Path, sample_memories):
    """Running index twice skips unchanged files."""
    idx = MemoryIndex(vectordb_dir=vectordb_dir)
    idx.build(memories_dir)
    stats = idx.build(memories_dir)
    assert stats["new"] == 0
    assert stats["updated"] == 0
    assert stats["deleted"] == 0


def test_index_detects_new_file(memories_dir: Path, vectordb_dir: Path, sample_memories):
    """Adding a new memory file is detected on next index."""
    idx = MemoryIndex(vectordb_dir=vectordb_dir)
    idx.build(memories_dir)

    new_mem = Memory(
        id="ggg777hhh888",
        repo="backend",
        type="note",
        tags=["logging"],
        author="bob",
        created=datetime(2026, 3, 20, 16, 0, tzinfo=timezone.utc),
        content="We use structured JSON logging via structlog.",
    )
    write_memory(new_mem, memories_dir / "backend" / f"{new_mem.id}.md")
    stats = idx.build(memories_dir)
    assert stats["new"] == 1


def test_index_detects_deleted_file(memories_dir: Path, vectordb_dir: Path, sample_memories):
    """Deleting a memory file removes it from the index."""
    idx = MemoryIndex(vectordb_dir=vectordb_dir)
    idx.build(memories_dir)

    (memories_dir / "frontend" / "eee555fff666.md").unlink()
    stats = idx.build(memories_dir)
    assert stats["deleted"] == 1


def test_force_rebuild(memories_dir: Path, vectordb_dir: Path, sample_memories):
    """Force rebuild re-indexes everything."""
    idx = MemoryIndex(vectordb_dir=vectordb_dir)
    idx.build(memories_dir)
    stats = idx.build(memories_dir, force=True)
    assert stats["new"] == 3


def test_upsert_single_memory(vectordb_dir: Path, sample_memories):
    """Upserting a single memory adds it to the index."""
    idx = MemoryIndex(vectordb_dir=vectordb_dir)
    idx.upsert(sample_memories[0])
    assert idx.count() == 1
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `pytest tests/test_index.py -v`
Expected: FAIL — `ModuleNotFoundError`

- [ ] **Step 4: Implement index.py**

```python
# src/qwick_rag/index.py
"""LanceDB indexing: embed, upsert, delete, rebuild, optimize."""

import json
import logging
from pathlib import Path

import lancedb
from fastembed import TextEmbedding

from qwick_rag.errors import MemoryParseError, VectorIndexError
from qwick_rag.memory import Memory, parse_memory, scan_memories

logger = logging.getLogger(__name__)

TABLE_NAME = "memories"
MODEL_NAME = "sentence-transformers/all-MiniLM-L6-v2"
META_FILE = "meta.json"


class MemoryIndex:
    """Manages the LanceDB vector index for memories."""

    def __init__(self, vectordb_dir: Path):
        self.vectordb_dir = vectordb_dir
        self.vectordb_dir.mkdir(parents=True, exist_ok=True)
        self._db = lancedb.connect(str(vectordb_dir))
        self._model = None

    @property
    def model(self) -> TextEmbedding:
        if self._model is None:
            self._model = TextEmbedding(MODEL_NAME)
            self._save_meta()
        return self._model

    def _save_meta(self):
        """Save model metadata for version consistency checks."""
        meta_path = self.vectordb_dir / META_FILE
        meta_path.write_text(json.dumps({"model": MODEL_NAME}))

    def _embed(self, texts: list[str]) -> list[list[float]]:
        """Embed a list of texts and return vectors."""
        embeddings = list(self.model.embed(texts))
        return [e.tolist() for e in embeddings]

    def _get_table(self):
        """Get existing table or return None."""
        if TABLE_NAME in self._db.list_tables().tables:
            return self._db.open_table(TABLE_NAME)
        return None

    def _create_table(self, records: list[dict]):
        """Create a new table with records and FTS index for hybrid search."""
        table = self._db.create_table(TABLE_NAME, records, mode="overwrite")
        table.create_fts_index("content", replace=True)
        return table

    def upsert(self, memory: Memory) -> None:
        """Upsert a single memory into the index."""
        vector = self._embed([memory.content])[0]
        record = {
            "id": memory.id,
            "repo": memory.repo,
            "type": memory.type,
            "tags": ", ".join(memory.tags),
            "author": memory.author,
            "created": memory.created.isoformat(),
            "content": memory.content,
            "content_hash": memory.content_hash,
            "vector": vector,
        }
        table = self._get_table()
        if table is None:
            self._create_table([record])
        else:
            # Delete existing record with same ID if present
            try:
                table.delete(f'id = "{memory.id}"')
            except Exception:
                pass
            table.add([record])

    def delete(self, memory_id: str) -> None:
        """Delete a memory from the index by ID."""
        table = self._get_table()
        if table is not None:
            table.delete(f'id = "{memory_id}"')

    def count(self) -> int:
        """Return the number of entries in the index."""
        table = self._get_table()
        if table is None:
            return 0
        return table.count_rows()

    def build(self, memories_dir: Path, force: bool = False) -> dict[str, int]:
        """Build or incrementally update the index from memory files.

        Returns dict with keys: new, updated, deleted.
        """
        stats = {"new": 0, "updated": 0, "deleted": 0}

        # Scan all memory files
        files = scan_memories(memories_dir)
        file_memories: dict[str, Memory] = {}
        for f in files:
            try:
                mem = parse_memory(f)
                file_memories[mem.id] = mem
            except MemoryParseError as e:
                logger.warning("Skipping malformed memory: %s", e)
                continue

        table = self._get_table()

        if force or table is None:
            # Full rebuild
            if not file_memories:
                if table is not None:
                    self._db.drop_table(TABLE_NAME)
                return stats

            texts = [m.content for m in file_memories.values()]
            vectors = self._embed(texts)

            records = []
            for (mem_id, mem), vec in zip(file_memories.items(), vectors):
                records.append({
                    "id": mem.id,
                    "repo": mem.repo,
                    "type": mem.type,
                    "tags": ", ".join(mem.tags),
                    "author": mem.author,
                    "created": mem.created.isoformat(),
                    "content": mem.content,
                    "content_hash": mem.content_hash,
                    "vector": vec,
                })
            self._create_table(records)
            stats["new"] = len(records)
            return stats

        # Incremental update: compare content hashes
        existing_rows = table.to_arrow().to_pylist()
        existing_hashes = {}
        existing_ids = set()
        for row in existing_rows:
            existing_hashes[row["id"]] = row["content_hash"]
            existing_ids.add(row["id"])

        file_ids = set(file_memories.keys())

        # Detect new and updated
        to_upsert: list[Memory] = []
        for mem_id, mem in file_memories.items():
            if mem_id not in existing_ids:
                to_upsert.append(mem)
                stats["new"] += 1
            elif existing_hashes.get(mem_id) != mem.content_hash:
                to_upsert.append(mem)
                stats["updated"] += 1

        # Detect deleted (in index but not on disk)
        to_delete = existing_ids - file_ids
        stats["deleted"] = len(to_delete)

        # Apply changes
        for mem_id in to_delete:
            table.delete(f'id = "{mem_id}"')

        if to_upsert:
            texts = [m.content for m in to_upsert]
            vectors = self._embed(texts)
            records = []
            for mem, vec in zip(to_upsert, vectors):
                records.append({
                    "id": mem.id,
                    "repo": mem.repo,
                    "type": mem.type,
                    "tags": ", ".join(mem.tags),
                    "author": mem.author,
                    "created": mem.created.isoformat(),
                    "content": mem.content,
                    "content_hash": mem.content_hash,
                    "vector": vec,
                })
                # Delete before re-adding (for updates)
                try:
                    table.delete(f'id = "{mem.id}"')
                except Exception:
                    pass
            table.add(records)

        # Rebuild FTS index after changes
        if to_upsert or to_delete:
            try:
                table.create_fts_index("content", replace=True)
            except Exception:
                pass  # FTS rebuild is best-effort

        # Optimize
        try:
            table.optimize()
        except Exception:
            pass  # optimize is best-effort

        return stats
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `pytest tests/test_index.py -v`
Expected: 6 passed

Note: First run will download the embedding model (~30MB). Subsequent runs use cache.

- [ ] **Step 6: Commit**

```bash
git add src/qwick_rag/index.py tests/test_index.py tests/conftest.py
git commit -m "feat: add LanceDB indexing with incremental rebuild"
```

---

### Task 6: Search Pipeline

**Files:**
- Create: `src/qwick_rag/search.py`
- Create: `tests/test_search.py`

- [ ] **Step 1: Write the failing tests**

```python
# tests/test_search.py
from pathlib import Path

from qwick_rag.index import MemoryIndex
from qwick_rag.search import search_memories, SearchResult


def test_search_returns_results(memories_dir: Path, vectordb_dir: Path, sample_memories):
    """Searching after indexing returns relevant results."""
    idx = MemoryIndex(vectordb_dir=vectordb_dir)
    idx.build(memories_dir)

    results = search_memories(idx, "PostgreSQL database choice")
    assert len(results) > 0
    assert isinstance(results[0], SearchResult)
    # The postgres decision should be the top result
    assert "PostgreSQL" in results[0].content


def test_search_with_repo_filter(memories_dir: Path, vectordb_dir: Path, sample_memories):
    """Filtering by repo only returns memories from that repo."""
    idx = MemoryIndex(vectordb_dir=vectordb_dir)
    idx.build(memories_dir)

    results = search_memories(idx, "components", repo="frontend")
    assert len(results) > 0
    assert all(r.repo == "frontend" for r in results)


def test_search_with_type_filter(memories_dir: Path, vectordb_dir: Path, sample_memories):
    """Filtering by type only returns memories of that type."""
    idx = MemoryIndex(vectordb_dir=vectordb_dir)
    idx.build(memories_dir)

    results = search_memories(idx, "session tokens", type_filter="bug")
    assert len(results) > 0
    assert all(r.type == "bug" for r in results)


def test_search_empty_index(vectordb_dir: Path):
    """Searching an empty index returns no results."""
    idx = MemoryIndex(vectordb_dir=vectordb_dir)
    results = search_memories(idx, "anything")
    assert results == []


def test_search_result_has_score(memories_dir: Path, vectordb_dir: Path, sample_memories):
    """Search results include a relevance score."""
    idx = MemoryIndex(vectordb_dir=vectordb_dir)
    idx.build(memories_dir)

    results = search_memories(idx, "database")
    assert results[0].score > 0
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `pytest tests/test_search.py -v`
Expected: FAIL — `ModuleNotFoundError`

- [ ] **Step 3: Implement search.py**

```python
# src/qwick_rag/search.py
"""Query pipeline: hybrid search (vector + BM25), metadata filtering, results."""

import logging
from dataclasses import dataclass

from qwick_rag.index import MemoryIndex

logger = logging.getLogger(__name__)


@dataclass
class SearchResult:
    """A single search result."""

    id: str
    repo: str
    type: str
    tags: str
    author: str
    created: str
    content: str
    score: float


def search_memories(
    index: MemoryIndex,
    query: str,
    repo: str | None = None,
    type_filter: str | None = None,
    tag: str | None = None,
    limit: int = 10,
) -> list[SearchResult]:
    """Search memories using hybrid search (vector + BM25 with RRF reranking).

    Filters are pushed into retrieval (applied before search, not after).
    """
    table = index._get_table()
    if table is None:
        return []

    # Embed the query for vector search
    query_vector = index._embed([query])[0]

    # Build hybrid search: vector similarity + BM25 full-text
    search = (
        table.search(query_type="hybrid")
        .vector(query_vector)
        .text(query)
    )

    # Push metadata filters into retrieval
    where_clauses = []
    if repo:
        where_clauses.append(f'repo = "{repo}"')
    if type_filter:
        where_clauses.append(f'type = "{type_filter}"')
    if tag:
        where_clauses.append(f'tags LIKE "%{tag}%"')

    if where_clauses:
        search = search.where(" AND ".join(where_clauses))

    search = search.limit(limit)

    try:
        results_list = search.to_list()
    except Exception as e:
        logger.error("Search failed: %s", e)
        return []

    results = []
    for row in results_list:
        results.append(SearchResult(
            id=row["id"],
            repo=row["repo"],
            type=row["type"],
            tags=row.get("tags", ""),
            author=row.get("author", "unknown"),
            created=row.get("created", ""),
            content=row["content"],
            score=float(row.get("_relevance_score", 0)),
        ))
    return results
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `pytest tests/test_search.py -v`
Expected: 5 passed

- [ ] **Step 5: Commit**

```bash
git add src/qwick_rag/search.py tests/test_search.py
git commit -m "feat: add vector search with metadata filtering"
```

---

### Task 7: CLI Commands

**Files:**
- Create: `src/qwick_rag/cli.py`
- Create: `tests/test_cli.py`

- [ ] **Step 1: Write the failing tests**

```python
# tests/test_cli.py
import os
from pathlib import Path

from typer.testing import CliRunner

from qwick_rag.cli import app

runner = CliRunner()


def test_save_creates_memory_file(tmp_path: Path, monkeypatch):
    """'save' command creates a markdown file in memories/."""
    rag_dir = tmp_path / "rag"
    rag_dir.mkdir()
    (rag_dir / "memories").mkdir()
    monkeypatch.setenv("QWICK_MEMORY_DIR", str(rag_dir))
    # Fake git context
    monkeypatch.setenv("QWICK_MEMORY_REPO", "test-repo")
    monkeypatch.setenv("QWICK_MEMORY_AUTHOR", "tester")

    result = runner.invoke(app, ["save", "Test memory content", "--type", "note"])
    assert result.exit_code == 0, result.output
    # Check file was created
    files = list((rag_dir / "memories" / "test-repo").glob("*.md"))
    assert len(files) == 1


def test_search_returns_results(tmp_path: Path, monkeypatch):
    """'search' command finds saved memories."""
    rag_dir = tmp_path / "rag"
    rag_dir.mkdir()
    (rag_dir / "memories").mkdir()
    monkeypatch.setenv("QWICK_MEMORY_DIR", str(rag_dir))
    monkeypatch.setenv("QWICK_MEMORY_REPO", "test-repo")
    monkeypatch.setenv("QWICK_MEMORY_AUTHOR", "tester")

    # Save a memory first
    runner.invoke(app, ["save", "We use PostgreSQL for our database", "--type", "decision"])

    result = runner.invoke(app, ["search", "database"])
    assert result.exit_code == 0
    assert "PostgreSQL" in result.output


def test_list_shows_memories(tmp_path: Path, monkeypatch):
    """'list' command shows saved memories."""
    rag_dir = tmp_path / "rag"
    rag_dir.mkdir()
    (rag_dir / "memories").mkdir()
    monkeypatch.setenv("QWICK_MEMORY_DIR", str(rag_dir))
    monkeypatch.setenv("QWICK_MEMORY_REPO", "test-repo")
    monkeypatch.setenv("QWICK_MEMORY_AUTHOR", "tester")

    runner.invoke(app, ["save", "A test memory", "--type", "note"])
    result = runner.invoke(app, ["list"])
    assert result.exit_code == 0
    assert "test-repo" in result.output


def test_delete_removes_memory(tmp_path: Path, monkeypatch):
    """'delete' command removes a memory file and index entry."""
    rag_dir = tmp_path / "rag"
    rag_dir.mkdir()
    (rag_dir / "memories").mkdir()
    monkeypatch.setenv("QWICK_MEMORY_DIR", str(rag_dir))
    monkeypatch.setenv("QWICK_MEMORY_REPO", "test-repo")
    monkeypatch.setenv("QWICK_MEMORY_AUTHOR", "tester")

    runner.invoke(app, ["save", "Memory to delete", "--type", "note"])
    files = list((rag_dir / "memories" / "test-repo").glob("*.md"))
    assert len(files) == 1
    mem_id = files[0].stem

    result = runner.invoke(app, ["delete", mem_id])
    assert result.exit_code == 0
    assert not files[0].exists()


def test_index_command(tmp_path: Path, monkeypatch):
    """'index' command rebuilds the vector index."""
    rag_dir = tmp_path / "rag"
    rag_dir.mkdir()
    (rag_dir / "memories").mkdir()
    monkeypatch.setenv("QWICK_MEMORY_DIR", str(rag_dir))

    result = runner.invoke(app, ["index"])
    assert result.exit_code == 0
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `pytest tests/test_cli.py -v`
Expected: FAIL — `ModuleNotFoundError`

- [ ] **Step 3: Implement cli.py**

```python
# src/qwick_rag/cli.py
"""Typer CLI commands: save, search, list, delete, index, doctor."""

import logging
import os
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Optional

import typer
from rich.console import Console
from rich.table import Table

from qwick_rag.config import get_rag_dir, get_memories_dir, get_vectordb_dir, get_repo, get_author, get_index
from qwick_rag.errors import QwickRagError, StorageError
from qwick_rag.index import MemoryIndex
from qwick_rag.memory import (
    Memory,
    generate_id,
    parse_memory,
    scan_memories,
    write_memory,
    MEMORY_TYPES,
)
from qwick_rag.search import search_memories

logger = logging.getLogger(__name__)
console = Console(stderr=True)
out = Console()

app = typer.Typer(
    name="qwick-memory",
    help="Centralized RAG memory for multiple repositories.",
    no_args_is_help=True,
)

TOKEN_WARN_LIMIT = 180  # ~256 tokens ≈ ~180 words for all-MiniLM-L6-v2


@app.command()
def save(
    content: Optional[str] = typer.Argument(None, help="Memory content. Omit to open $EDITOR."),
    type: str = typer.Option("note", "--type", "-t", help=f"Memory type: {', '.join(MEMORY_TYPES)}"),
    tags: str = typer.Option("", "--tags", help="Comma-separated tags"),
):
    """Save a new memory."""
    if content is None:
        content = typer.edit("")
        if not content or not content.strip():
            console.print("[red]No content provided. Aborting.[/red]")
            raise typer.Exit(1)
    content = content.strip()

    if type not in MEMORY_TYPES:
        console.print(f"[red]Invalid type '{type}'. Must be one of: {', '.join(MEMORY_TYPES)}[/red]")
        raise typer.Exit(1)

    # Warn on long content
    word_count = len(content.split())
    if word_count > TOKEN_WARN_LIMIT:
        console.print(f"[yellow]Warning: content is ~{word_count} words. Embeddings may only capture the first ~256 tokens.[/yellow]")

    tag_list = [t.strip() for t in tags.split(",") if t.strip()] if tags else []
    mem_id = generate_id(content)
    repo = get_repo()
    author = get_author()

    memory = Memory(
        id=mem_id,
        repo=repo,
        type=type,
        tags=tag_list,
        author=author,
        created=datetime.now(timezone.utc),
        content=content,
    )

    memories_dir = get_memories_dir()
    filepath = memories_dir / repo / f"{mem_id}.md"

    if filepath.exists():
        console.print(f"[yellow]Memory already exists: {mem_id}[/yellow]")
        raise typer.Exit(0)

    # Atomic save: temp file -> embed -> upsert -> rename
    tmp_path = filepath.parent / f".tmp_{mem_id}.md"
    try:
        write_memory(memory, tmp_path)
        idx = get_index()
        idx.upsert(memory)
        tmp_path.rename(filepath)
    except Exception as e:
        tmp_path.unlink(missing_ok=True)
        console.print(f"[red]Save failed: {e}[/red]")
        raise typer.Exit(1)

    out.print(f"Saved memory [bold]{mem_id}[/bold] to {repo}/")


@app.command()
def search(
    query: str = typer.Argument(..., help="Search query"),
    repo: Optional[str] = typer.Option(None, "--repo", "-r", help="Filter by repo"),
    type: Optional[str] = typer.Option(None, "--type", "-t", help="Filter by type"),
    tag: Optional[str] = typer.Option(None, "--tag", help="Filter by tag"),
    limit: int = typer.Option(10, "--limit", "-l", help="Max results"),
):
    """Search memories using semantic similarity."""
    idx = get_index()
    results = search_memories(idx, query, repo=repo, type_filter=type, tag=tag, limit=limit)

    if not results:
        out.print("No results found.")
        raise typer.Exit(0)

    table = Table(title="Search Results")
    table.add_column("Score", style="cyan", width=8)
    table.add_column("Repo", style="green")
    table.add_column("Type", style="magenta")
    table.add_column("Content", max_width=60)
    table.add_column("ID", style="dim")

    for r in results:
        preview = r.content[:80] + "..." if len(r.content) > 80 else r.content
        table.add_row(f"{r.score:.4f}", r.repo, r.type, preview, r.id)

    out.print(table)


@app.command(name="list")
def list_memories(
    repo: Optional[str] = typer.Option(None, "--repo", "-r", help="Filter by repo"),
    type: Optional[str] = typer.Option(None, "--type", "-t", help="Filter by type"),
    tags: Optional[str] = typer.Option(None, "--tags", help="Filter by tag"),
):
    """List memories with optional filters."""
    memories_dir = get_memories_dir()
    files = scan_memories(memories_dir)

    table = Table(title="Memories")
    table.add_column("ID", style="dim")
    table.add_column("Repo", style="green")
    table.add_column("Type", style="magenta")
    table.add_column("Tags", style="blue")
    table.add_column("Content", max_width=50)

    count = 0
    for f in files:
        try:
            mem = parse_memory(f)
        except Exception:
            continue

        if repo and mem.repo != repo:
            continue
        if type and mem.type != type:
            continue
        if tags and tags not in mem.tags:
            continue

        preview = mem.content[:50] + "..." if len(mem.content) > 50 else mem.content
        table.add_row(mem.id, mem.repo, mem.type, ", ".join(mem.tags), preview)
        count += 1

    out.print(table)
    out.print(f"\n{count} memories found.")


@app.command()
def delete(
    memory_id: str = typer.Argument(..., help="Memory ID to delete"),
):
    """Delete a memory by ID."""
    memories_dir = get_memories_dir()
    # Find the file
    matches = list(memories_dir.rglob(f"{memory_id}.md"))
    if not matches:
        console.print(f"[red]Memory {memory_id} not found.[/red]")
        raise typer.Exit(1)

    filepath = matches[0]
    filepath.unlink()

    idx = get_index()
    idx.delete(memory_id)

    out.print(f"Deleted memory [bold]{memory_id}[/bold]")


@app.command()
def index(
    force: bool = typer.Option(False, "--force", help="Full rebuild (drop and re-index everything)"),
):
    """Rebuild the vector index from memory files."""
    idx = get_index()
    memories_dir = get_memories_dir()

    if force:
        out.print("Force rebuilding index from scratch...")
    else:
        out.print("Incrementally updating index...")

    stats = idx.build(memories_dir, force=force)
    out.print(f"Indexed {stats['new']} new, {stats['updated']} updated, {stats['deleted']} deleted.")


@app.command()
def doctor():
    """Run diagnostics on qwick-memory setup."""
    rag_dir = get_rag_dir()
    memories_dir = get_memories_dir()
    vectordb_dir = get_vectordb_dir()

    checks_passed = 0
    checks_failed = 0

    # Check memories dir
    if memories_dir.exists():
        out.print("[green]✓[/green] memories/ directory exists")
        checks_passed += 1
    else:
        out.print("[red]✗[/red] memories/ directory not found")
        checks_failed += 1

    # Check memory files
    files = scan_memories(memories_dir)
    malformed = 0
    for f in files:
        try:
            parse_memory(f)
        except Exception:
            malformed += 1
    if malformed == 0:
        out.print(f"[green]✓[/green] {len(files)} memory files, all valid")
        checks_passed += 1
    else:
        out.print(f"[yellow]![/yellow] {malformed}/{len(files)} memory files malformed")
        checks_failed += 1

    # Check vectordb
    try:
        idx = MemoryIndex(vectordb_dir=vectordb_dir)
        count = idx.count()
        out.print(f"[green]✓[/green] .vectordb/ healthy, {count} entries indexed")
        checks_passed += 1
    except Exception as e:
        out.print(f"[red]✗[/red] .vectordb/ error: {e}")
        checks_failed += 1

    # Check index consistency
    if vectordb_dir.exists() and memories_dir.exists():
        file_count = len(files) - malformed
        if count == file_count:
            out.print(f"[green]✓[/green] Index consistent ({count} entries = {file_count} files)")
            checks_passed += 1
        else:
            out.print(f"[yellow]![/yellow] Index inconsistent: {count} entries vs {file_count} files. Run 'qwick-memory index'")
            checks_failed += 1

    # Check model meta
    meta_path = vectordb_dir / "meta.json"
    if meta_path.exists():
        import json
        meta = json.loads(meta_path.read_text())
        from qwick_rag.index import MODEL_NAME
        if meta.get("model") == MODEL_NAME:
            out.print(f"[green]✓[/green] Embedding model consistent: {MODEL_NAME}")
            checks_passed += 1
        else:
            out.print(f"[red]✗[/red] Model mismatch: index used {meta.get('model')}, current is {MODEL_NAME}. Run 'qwick-memory index --force'")
            checks_failed += 1
    else:
        out.print("[yellow]![/yellow] No model metadata found (not yet indexed?)")

    # Check git context
    repo = detect_repo_name()
    author = detect_author()
    out.print(f"[green]✓[/green] Git context: repo={repo}, author={author}")
    checks_passed += 1

    out.print(f"\n{checks_passed} passed, {checks_failed} failed")
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `pytest tests/test_cli.py -v`
Expected: 5 passed

- [ ] **Step 5: Verify CLI works end-to-end**

```bash
export QWICK_MEMORY_DIR=/Users/falconiere/Projects/qwick-memory
qwick-memory save "Test memory from CLI" --type note
qwick-memory search "test"
qwick-memory list
qwick-memory doctor
```

- [ ] **Step 6: Commit**

```bash
git add src/qwick_rag/cli.py tests/test_cli.py
git commit -m "feat: add CLI commands (save, search, list, delete, index, doctor)"
```

---

### Task 8: MCP Server

**Files:**
- Create: `src/qwick_rag/server.py`
- Create: `tests/test_server.py`

- [ ] **Step 1: Write the failing tests**

```python
# tests/test_server.py
import os
import pytest
from pathlib import Path
from unittest.mock import patch

from qwick_rag.server import rag_save, rag_search, rag_index


@pytest.mark.asyncio
async def test_rag_save(tmp_path: Path):
    """MCP rag_save tool creates a memory."""
    rag_dir = tmp_path / "rag"
    rag_dir.mkdir()
    (rag_dir / "memories").mkdir()

    with patch.dict(os.environ, {
        "QWICK_MEMORY_DIR": str(rag_dir),
        "QWICK_MEMORY_REPO": "test-repo",
        "QWICK_MEMORY_AUTHOR": "tester",
    }):
        result = await rag_save(
            content="We use Redis for caching",
            type="decision",
            tags="redis,caching",
        )
    assert "Saved" in result


@pytest.mark.asyncio
async def test_rag_search(tmp_path: Path):
    """MCP rag_search finds saved memories."""
    rag_dir = tmp_path / "rag"
    rag_dir.mkdir()
    (rag_dir / "memories").mkdir()

    with patch.dict(os.environ, {
        "QWICK_MEMORY_DIR": str(rag_dir),
        "QWICK_MEMORY_REPO": "test-repo",
        "QWICK_MEMORY_AUTHOR": "tester",
    }):
        await rag_save(content="PostgreSQL is our primary database", type="decision", tags="db")
        result = await rag_search(query="database")
    assert "PostgreSQL" in result


@pytest.mark.asyncio
async def test_rag_index(tmp_path: Path):
    """MCP rag_index rebuilds the index."""
    rag_dir = tmp_path / "rag"
    rag_dir.mkdir()
    (rag_dir / "memories").mkdir()

    with patch.dict(os.environ, {"QWICK_MEMORY_DIR": str(rag_dir)}):
        result = await rag_index(force=False)
    assert "Indexed" in result
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `pytest tests/test_server.py -v`
Expected: FAIL — `ModuleNotFoundError`

- [ ] **Step 3: Implement server.py**

```python
# src/qwick_rag/server.py
"""MCP server exposing qwick-memory tools for Claude Code."""

import logging
import os
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Optional

from mcp.server.fastmcp import FastMCP

from qwick_rag.config import get_memories_dir, get_repo, get_author, get_index
from qwick_rag.errors import QwickRagError
from qwick_rag.memory import Memory, generate_id, parse_memory, scan_memories, write_memory
from qwick_rag.search import search_memories as _search

# Configure logging to stderr (required by MCP protocol)
logging.basicConfig(stream=sys.stderr, level=logging.INFO)
logger = logging.getLogger(__name__)

mcp = FastMCP("qwick-memory")


@mcp.tool()
async def rag_save(content: str, type: str = "note", tags: str = "") -> str:
    """Save a memory observation.

    Args:
        content: The memory content to save.
        type: Memory type (decision, bug, convention, discovery, pattern, preference, note).
        tags: Comma-separated tags for filtering.
    """
    tag_list = [t.strip() for t in tags.split(",") if t.strip()] if tags else []
    mem_id = generate_id(content)
    repo = get_repo()
    author = get_author()

    memory = Memory(
        id=mem_id,
        repo=repo,
        type=type,
        tags=tag_list,
        author=author,
        created=datetime.now(timezone.utc),
        content=content,
    )

    memories_dir = get_memories_dir()
    filepath = memories_dir / repo / f"{mem_id}.md"

    if filepath.exists():
        return f"Memory already exists: {mem_id}"

    tmp_path = filepath.parent / f".tmp_{mem_id}.md"
    try:
        write_memory(memory, tmp_path)
        idx = get_index()
        idx.upsert(memory)
        tmp_path.rename(filepath)
    except Exception as e:
        tmp_path.unlink(missing_ok=True)
        return f"Save failed: {e}"

    return f"Saved memory {mem_id} to {repo}/"


@mcp.tool()
async def rag_search(
    query: str,
    repo: str | None = None,
    type: str | None = None,
    tag: str | None = None,
    limit: int = 10,
) -> str:
    """Search memories using semantic similarity.

    Args:
        query: Search query string.
        repo: Filter by repository name.
        type: Filter by memory type.
        tag: Filter by tag.
        limit: Maximum number of results.
    """
    idx = get_index()
    results = _search(idx, query, repo=repo, type_filter=type, tag=tag, limit=limit)

    if not results:
        return "No results found."

    lines = []
    for r in results:
        lines.append(f"[{r.score:.4f}] {r.repo}/{r.id} ({r.type})")
        if r.tags:
            lines.append(f"  Tags: {r.tags}")
        lines.append(f"  {r.content}")
        lines.append("")
    return "\n".join(lines)


@mcp.tool()
async def rag_list(repo: str | None = None, type: str | None = None) -> str:
    """List memories with optional filters.

    Args:
        repo: Filter by repository name.
        type: Filter by memory type.
    """
    memories_dir = get_memories_dir()
    files = scan_memories(memories_dir)

    entries = []
    for f in files:
        try:
            mem = parse_memory(f)
        except Exception:
            continue
        if repo and mem.repo != repo:
            continue
        if type and mem.type != type:
            continue
        preview = mem.content[:80] + "..." if len(mem.content) > 80 else mem.content
        entries.append(f"{mem.repo}/{mem.id} ({mem.type}): {preview}")

    if not entries:
        return "No memories found."
    return "\n".join(entries)


@mcp.tool()
async def rag_delete(memory_id: str) -> str:
    """Delete a memory by ID.

    Args:
        memory_id: The ID of the memory to delete.
    """
    memories_dir = get_memories_dir()
    matches = list(memories_dir.rglob(f"{memory_id}.md"))
    if not matches:
        return f"Memory {memory_id} not found."

    matches[0].unlink()
    idx = get_index()
    idx.delete(memory_id)
    return f"Deleted memory {memory_id}"


@mcp.tool()
async def rag_index(force: bool = False) -> str:
    """Rebuild the vector index from memory files.

    Args:
        force: If True, drop and rebuild from scratch.
    """
    idx = get_index()
    memories_dir = get_memories_dir()
    stats = idx.build(memories_dir, force=force)
    return f"Indexed {stats['new']} new, {stats['updated']} updated, {stats['deleted']} deleted."


@mcp.tool()
async def rag_context(repo: str | None = None, limit: int = 20) -> str:
    """Get recent memories for the current repo. Useful for session start context loading.

    Args:
        repo: Repository name. Auto-detected if not provided.
        limit: Maximum number of memories to return.
    """
    repo = repo or get_repo()
    memories_dir = get_memories_dir()
    files = scan_memories(memories_dir)

    entries = []
    for f in files:
        try:
            mem = parse_memory(f)
        except Exception:
            continue
        if mem.repo != repo:
            continue
        entries.append(mem)

    # Sort by created descending, take top N
    entries.sort(key=lambda m: m.created, reverse=True)
    entries = entries[:limit]

    if not entries:
        return f"No memories found for repo '{repo}'."

    lines = []
    for mem in entries:
        lines.append(f"[{mem.type}] {mem.content}")
        if mem.tags:
            lines.append(f"  Tags: {', '.join(mem.tags)}")
        lines.append("")
    return "\n".join(lines)


def main():
    """Run the MCP server."""
    mcp.run(transport="stdio")


if __name__ == "__main__":
    main()
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `pytest tests/test_server.py -v`
Expected: 3 passed

- [ ] **Step 5: Commit**

```bash
git add src/qwick_rag/server.py tests/test_server.py
git commit -m "feat: add MCP server with rag tools for Claude Code"
```

---

### Task 9: Claude Code Plugin Files

**Files:**
- Create: `.claude-plugin/plugin.json`
- Create: `.claude-plugin/marketplace.json`
- Create: `.mcp.json`
- Create: `hooks/hooks.json`
- Create: `scripts/session-start.sh`
- Create: `skills/memory/SKILL.md`

- [ ] **Step 1: Create plugin manifest**

```json
{
  "name": "qwick-memory",
  "description": "Centralized RAG memory for multiple repositories",
  "version": "0.1.0",
  "author": { "name": "SidegigLLC" },
  "repository": "https://github.com/SidegigLLC/qwick-memory",
  "license": "MIT"
}
```

- [ ] **Step 2: Create marketplace metadata**

```json
{
  "$schema": "https://anthropic.com/claude-code/marketplace.schema.json",
  "name": "qwick-memory",
  "description": "Centralized RAG memory for multiple repositories",
  "plugins": [
    {
      "source": "."
    }
  ]
}
```

- [ ] **Step 3: Create MCP server config**

```json
{
  "mcpServers": {
    "qwick-memory": {
      "command": "uv",
      "args": ["run", "--directory", "${CLAUDE_PLUGIN_ROOT}", "python", "-m", "qwick_rag.server"]
    }
  }
}
```

- [ ] **Step 4: Create hooks config**

```json
{
  "hooks": [
    {
      "event": "SessionStart",
      "commands": [
        {
          "command": "${CLAUDE_PLUGIN_ROOT}/scripts/session-start.sh",
          "timeout": 30000
        }
      ]
    }
  ]
}
```

- [ ] **Step 5: Create session-start script**

```bash
#!/usr/bin/env bash
# scripts/session-start.sh — Auto-index on session start
set -euo pipefail

PLUGIN_ROOT="${CLAUDE_PLUGIN_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
cd "$PLUGIN_ROOT"

# Rebuild index if memories exist
if [ -d "memories" ]; then
  uv run python -m qwick_rag index 2>/dev/null || true
fi
```

Make executable: `chmod +x scripts/session-start.sh`

- [ ] **Step 6: Create memory skill**

```markdown
<!-- skills/memory/SKILL.md -->
---
name: memory
description: ALWAYS ACTIVE — Centralized memory protocol for cross-repository knowledge. Save decisions, bugs, conventions, and discoveries proactively.
---

## qwick-memory Memory Protocol

You have qwick-memory memory tools (rag_save, rag_search, rag_list, rag_delete, rag_index, rag_context).

### PROACTIVE SAVE — do NOT wait for user to ask
Call `rag_save` IMMEDIATELY after ANY of these:
- Decision made (architecture, convention, workflow, tool choice)
- Bug fixed (include root cause)
- Convention or workflow established
- Non-obvious discovery or edge case found
- Pattern established (naming, structure, approach)

### SEARCH MEMORY when:
- Starting work on something that might have been done before
- User asks to recall anything
- User mentions a topic you have no context on
- User's first message references a problem or feature

### Memory Types
- `decision` — Architecture, tool, or workflow choices
- `bug` — Bug root causes and fixes
- `convention` — Coding standards, naming patterns
- `discovery` — Non-obvious findings, gotchas
- `pattern` — Established approaches
- `preference` — User or team preferences
- `note` — General knowledge that doesn't fit other types
```

- [ ] **Step 7: Commit**

```bash
git add .claude-plugin/ .mcp.json hooks/ scripts/ skills/
git commit -m "feat: add Claude Code plugin files (manifest, MCP config, hooks, skill)"
```

---

### Task 10: Update Project Files

**Files:**
- Modify: `AGENTS.md`
- Modify: `README.md`
- Modify: `.gitignore`

- [ ] **Step 1: Update AGENTS.md with build/test/dev commands**

Replace the "Build, Test, and Development Commands" section with actual commands now that code exists:

```markdown
## Build, Test, and Development Commands
- `uv pip install -e ".[dev]"` — install in dev mode with test dependencies
- `pytest` — run all tests
- `pytest tests/test_memory.py -v` — run a specific test file
- `pytest -k test_name` — run a specific test
- `ruff check src/ tests/` — lint
- `ruff format src/ tests/` — format (2-space indent)
- `ruff format --check src/ tests/` — verify formatting
- `pyright src/` — type checking
- `qwick-memory --help` — show CLI help
- `qwick-memory doctor` — run diagnostics
```

- [ ] **Step 2: Update README.md with installation and usage**

Add a quick start section covering `pip install`, basic save/search/list commands, and Claude Code plugin installation.

- [ ] **Step 3: Ensure .gitignore has .vectordb/**

Verify `.vectordb/` is in `.gitignore` (added in Task 1).

- [ ] **Step 4: Commit**

```bash
git add AGENTS.md README.md .gitignore
git commit -m "docs: update project files with build commands and usage"
```

---

### Task 11: Quality Gate (Lint, Format, Type Check)

**Files:**
- All `src/` and `tests/` files

This task formats ALL code to 2-space indentation, fixes lint issues, and verifies types. Run after all implementation is complete.

- [ ] **Step 1: Format all code with ruff (2-space indent)**

```bash
ruff format src/ tests/
```

- [ ] **Step 2: Fix lint issues**

```bash
ruff check --fix src/ tests/
```

Review any remaining issues that can't be auto-fixed and fix manually.

- [ ] **Step 3: Run ruff check (no auto-fix) to verify clean**

```bash
ruff check src/ tests/
```

Expected: No errors

- [ ] **Step 4: Run pyright type checking**

```bash
pyright src/
```

Fix any type errors. Common fixes:
- Add type annotations to function signatures
- Use `from __future__ import annotations` if needed for `X | None` syntax on Python 3.10
- Add `# type: ignore` only as last resort with comment explaining why

- [ ] **Step 5: Run tests to make sure formatting didn't break anything**

```bash
pytest -v
```

Expected: All tests pass

- [ ] **Step 6: Build package to verify it's installable**

```bash
uv build
```

Expected: Creates `dist/qwick_rag-0.1.0-py3-none-any.whl` and `.tar.gz`

- [ ] **Step 7: Commit**

```bash
git add src/ tests/
git commit -m "style: format all code with ruff (2-space indent) and fix type issues"
```

---

### Task 12: End-to-End Integration Test

**Files:**
- Create: `tests/test_integration.py`

- [ ] **Step 1: Write integration test**

```python
# tests/test_integration.py
"""End-to-end integration test: save → index → search → delete → index."""

import os
from pathlib import Path

from typer.testing import CliRunner

from qwick_rag.cli import app

runner = CliRunner()


def test_full_lifecycle(tmp_path: Path, monkeypatch):
    """Test the complete memory lifecycle: save, search, list, delete."""
    rag_dir = tmp_path / "rag"
    rag_dir.mkdir()
    (rag_dir / "memories").mkdir()
    monkeypatch.setenv("QWICK_MEMORY_DIR", str(rag_dir))
    monkeypatch.setenv("QWICK_MEMORY_REPO", "integration-test")
    monkeypatch.setenv("QWICK_MEMORY_AUTHOR", "tester")

    # Save three memories
    runner.invoke(app, ["save", "PostgreSQL is our primary database", "--type", "decision", "--tags", "db,postgres"])
    runner.invoke(app, ["save", "Redis handles session caching", "--type", "decision", "--tags", "cache,redis"])
    runner.invoke(app, ["save", "Always use named exports in React", "--type", "convention", "--tags", "react"])

    # List should show 3
    result = runner.invoke(app, ["list"])
    assert result.exit_code == 0
    assert "3 memories found" in result.output

    # Search should find postgres memory
    result = runner.invoke(app, ["search", "which database do we use"])
    assert result.exit_code == 0
    assert "PostgreSQL" in result.output

    # Search with type filter
    result = runner.invoke(app, ["search", "react components", "--type", "convention"])
    assert result.exit_code == 0
    assert "named exports" in result.output

    # Delete one memory
    files = list((rag_dir / "memories" / "integration-test").glob("*.md"))
    first_id = files[0].stem
    result = runner.invoke(app, ["delete", first_id])
    assert result.exit_code == 0

    # List should show 2
    result = runner.invoke(app, ["list"])
    assert "2 memories found" in result.output

    # Rebuild index
    result = runner.invoke(app, ["index"])
    assert result.exit_code == 0

    # Doctor should pass
    result = runner.invoke(app, ["doctor"])
    assert result.exit_code == 0
```

- [ ] **Step 2: Run integration test**

Run: `pytest tests/test_integration.py -v`
Expected: PASS

- [ ] **Step 3: Run full test suite**

Run: `pytest -v`
Expected: All tests pass

- [ ] **Step 4: Commit**

```bash
git add tests/test_integration.py
git commit -m "test: add end-to-end integration test"
```

---

## Task Dependency Order

```
Task 1 (scaffold) → Task 2 (errors) → Task 3 (git utils) → Task 3b (config)
  → Task 4 (memory model) → Task 5 (indexing) → Task 6 (search)
  → Task 7 (CLI) → Task 8 (MCP server) → Task 9 (plugin files)
  → Task 10 (update docs) → Task 11 (quality gate) → Task 12 (integration test)
```

Each task builds on the previous. All tasks produce working, testable code independently.

## Code Quality Rules

**IMPORTANT — applies to ALL code written in every task:**

- **2-space indentation** (configured via `ruff.indent-width = 2` in pyproject.toml). All Python code in this project uses 2 spaces, not 4.
- After writing code in any task, run `ruff format src/ tests/` before committing.
- After implementing, run `ruff check src/ tests/` and fix any lint issues before committing.
- Task 11 is the final quality gate that ensures everything passes lint, format, type check, and build.
