# CLAUDE.md

## Project Overview

qwick-rag is a centralized RAG memory system for multiple repositories. It's a Python CLI tool and Claude Code MCP plugin that stores developer knowledge (decisions, bugs, conventions, discoveries) as markdown files with vector search.

## Architecture

- **Source of truth:** Markdown files with YAML frontmatter in `memories/{repo}/{id}.md`
- **Vector index:** LanceDB embedded (local, file-based, gitignored at `.vectordb/`)
- **Embeddings:** fastembed with `all-MiniLM-L6-v2` (ONNX, local, ~30MB model)
- **Search:** Vector similarity with BM25 fallback, metadata filtering
- **Interfaces:** Typer CLI (`qwick-rag`) + MCP server (FastMCP) for Claude Code
- **Sharing:** Git push/pull for markdown files, each dev rebuilds local index

## Key Commands

```bash
uv pip install -e ".[dev]"    # Install
pytest                         # Unit + integration tests (35 tests)
./scripts/e2e-test.sh          # Real CLI end-to-end test (26 checks)
./scripts/e2e-test.sh --build  # Install from source + run e2e
ruff format src/ tests/        # Format (2-space indent!)
ruff check src/ tests/         # Lint
pyright src/                   # Type check
qwick-rag doctor               # Health check
```

## Code Style

- **2-space indentation** — not 4. Enforced by ruff config in pyproject.toml.
- Line length 100.
- Type annotations on public functions.
- Imports: use `from __future__ import annotations` where needed for `X | None` syntax.

## Module Map

| Module | Responsibility |
|--------|---------------|
| `cli.py` | Typer CLI commands (save, search, list, delete, index, doctor) |
| `server.py` | MCP server with 6 tools for Claude Code |
| `memory.py` | Memory dataclass, markdown I/O, ID generation (SHA-256) |
| `index.py` | LanceDB: embed, upsert, delete, incremental rebuild, FTS index |
| `search.py` | Hybrid search with metadata filtering |
| `config.py` | Shared helpers (paths, repo/author detection from env or git) |
| `git_utils.py` | Auto-detect repo name and author from git remote/config |
| `errors.py` | QwickRagError hierarchy (5 error types) |

## Environment Variables

| Variable | Purpose | Default |
|----------|---------|---------|
| `QWICK_RAG_DIR` | Root directory of qwick-rag repo | `cwd()` |
| `QWICK_RAG_REPO` | Override repo name | Auto-detected from git remote |
| `QWICK_RAG_AUTHOR` | Override author name | Auto-detected from git config |

## Memory Data Model

```yaml
---
id: a1b2c3d4e5f6       # SHA-256 of content, 12 hex chars
repo: qwick-backend     # Auto-detected from git remote
type: decision          # decision|bug|convention|discovery|pattern|preference|note
tags: [database, postgres]
author: falconiere      # Auto-detected from git config
created: 2026-03-20T14:30:00+00:00
content_hash: a1b2c3d4e5f6  # For incremental indexing
---

The actual memory content goes here as markdown body.
```

## Save Flow (Atomic)

1. Generate ID (SHA-256 of content)
2. Write markdown to temp file `memories/{repo}/.{id}.tmp`
3. Embed content via fastembed
4. Upsert into LanceDB
5. Atomic rename temp → final `memories/{repo}/{id}.md`
6. On failure: delete temp file, report error

## Testing

- `conftest.py` has shared fixtures: `memories_dir`, `vectordb_dir`, `sample_memories`
- CLI tests use `typer.testing.CliRunner` with `monkeypatch.setenv`
- MCP server tests call async tool functions directly (not the protocol layer)
- First test run downloads the embedding model (~30MB, cached at `~/.cache/fastembed/`)
- `scripts/e2e-test.sh` runs the real CLI binary end-to-end in an isolated temp directory (save, list, search, duplicate detection, delete, index rebuild, doctor)

## Claude Code Plugin

The `.claude-plugin/` directory contains the marketplace manifest and plugin config. To install as a Claude Code plugin:

```
claude plugin add --marketplace SidegigLLC/qwick-rag
```

The `marketplace.json` requires `owner` (object with `name`), and each plugin entry requires `name`, `description`, and `source`. See `.claude-plugin/marketplace.json` for the current schema.

## Design Spec

Full architecture and design decisions: `docs/superpowers/specs/2026-03-20-qwick-rag-design.md`
