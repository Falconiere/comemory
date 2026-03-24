# CLAUDE.md

## Project Overview

qwick-memory is a centralized RAG memory system for multiple repositories. It's a Python CLI tool and Claude Code MCP plugin that stores developer knowledge (decisions, bugs, conventions, discoveries) as markdown files with vector search.

## Architecture

- **Source of truth:** Markdown files with YAML frontmatter in `memories/{id}.md`
- **Vector index:** LanceDB embedded (local, file-based, gitignored at `.vectordb/`)
- **Embeddings:** fastembed with `nomic-embed-text-v1.5-Q` (ONNX, local, ~130MB model, 768 dims, 8192 token context)
- **Search:** Vector similarity with BM25 fallback, metadata filtering
- **Interfaces:** Typer CLI (`qwick-memory`) + MCP server (FastMCP) for Claude Code
- **Sharing:** Git push/pull for markdown files, each dev rebuilds local index

## Key Commands

```bash
uv tool install -e ".[dev]"   # Install globally (puts qwick-memory + qwick-memory-server on PATH)
pytest                         # Unit + integration tests (49 tests)
./scripts/e2e-test.sh          # Real CLI end-to-end test (28 checks)
./scripts/e2e-test.sh --build  # Install from source + run e2e
ruff format src/ tests/        # Format (2-space indent!)
ruff check src/ tests/         # Lint
pyright src/                   # Type check
qwick-memory doctor            # Health check
```

## Code Style

- **2-space indentation** — not 4. Enforced by ruff config in pyproject.toml.
- Line length 100.
- Type annotations on public functions.
- Imports: use `from __future__ import annotations` where needed for `X | None` syntax.

## Module Map

| Module | Responsibility |
|--------|---------------|
| `cli.py` | Typer CLI commands (save, search, list, delete, index, context, doctor) |
| `server.py` | MCP server with 7 tools for Claude Code + memory protocol |
| `memory.py` | Memory dataclass, markdown I/O, ID generation (SHA-256) |
| `index.py` | LanceDB: embed, upsert, delete, incremental rebuild, FTS index |
| `search.py` | Hybrid search with metadata filtering |
| `config.py` | Shared helpers (paths, repo/author detection from env or git) |
| `git_utils.py` | Auto-detect repo name/author from git, auto-sync (commit+push) |
| `errors.py` | QwickRagError hierarchy (5 error types) |

## Environment Variables

| Variable | Purpose | Default |
|----------|---------|---------|
| `QWICK_MEMORY_DIR` | Root directory for memories and vectordb | `~/.qwick-memory/` |
| `QWICK_MEMORY_REPO` | Override repo name | Auto-detected from git remote |
| `QWICK_MEMORY_AUTHOR` | Override author name | Auto-detected from git config |
| `QWICK_MEMORY_REMOTE` | Override git remote URL (`""` to disable) | Auto-detected from source repo |

## Memory Data Model

```yaml
---
id: a1b2c3d4e5f6       # SHA-256 of content, 12 hex chars
repo: qwick-backend     # Auto-detected from git remote
type: decision          # decision|bug|convention|discovery|pattern|preference|note|session-summary
tags: [database, postgres]
author: falconiere      # Auto-detected from git config
created: 2026-03-20T14:30:00+00:00
content_hash: a1b2c3d4e5f6  # For incremental indexing
---

The actual memory content goes here as markdown body.
```

## Save Flow (Atomic)

1. Generate ID (SHA-256 of content)
2. Write markdown to temp file `memories/.{id}.tmp`
3. Embed content via fastembed
4. Upsert into LanceDB
5. Atomic rename temp → final `memories/{id}.md`
6. `git_sync`: add + commit + push (best-effort, never fails the save)
7. On failure (steps 2-5): delete temp file, report error

## Testing

- `conftest.py` has shared fixtures: `_reset_git_cache`, `sample_memories`, `memories_dir`
- CLI tests use `typer.testing.CliRunner` with `monkeypatch.setenv`
- MCP server tests call async tool functions directly (not the protocol layer)
- First test run downloads the embedding model (~130MB, cached at `~/.cache/fastembed/`)
- `scripts/e2e-test.sh` runs the real CLI binary end-to-end in an isolated temp directory (save, list, search, duplicate detection, delete, index rebuild, doctor)

## Claude Code Plugin

The `.claude-plugin/` directory contains the marketplace manifest and plugin config. To install as a Claude Code plugin:

```bash
claude plugin marketplace add SidegigLLC/qwick-memory
claude plugin install qwick-memory
```

The `marketplace.json` requires `owner` (object with `name`), and each plugin entry requires `name`, `description`, and `source`. See `.claude-plugin/marketplace.json` for the current schema.

## Scripts

All scripts in `scripts/` are **self-locating** — they resolve the project root from their own physical location via `dirname`, then use `uv run --directory` to find the package. This means they work from any working directory (critical for the plugin system, which may launch from a different project).

| Script | Purpose |
|--------|---------|
| `scripts/mcp-server.sh` | MCP server launcher (used by `.mcp.json`) |
| `scripts/session-start.sh` | Auto-index + load context (SessionStart hook) |
| `scripts/pre-compact.sh` | Reminder to save session summary (PreCompact hook) |
| `scripts/post-compact.sh` | Restore context after compaction (PostCompact hook) |
| `scripts/e2e-test.sh` | Real CLI end-to-end test (28 checks) |

Entry points in `pyproject.toml`:
- `qwick-memory` — CLI (`qwick_memory.cli:app`)
- `qwick-memory-server` — MCP server (`qwick_memory.server:main`)

## Memory Protocol

qwick-memory includes an automatic memory protocol injected via MCP server instructions. When active, Claude proactively saves decisions, bugs, conventions, discoveries, and session summaries. The protocol is defined in `server.py` as the `PROTOCOL` constant.

**Hooks:**
- `SessionStart` — Auto-index + load context
- `PreCompact` — Reminder to save session summary
- `PostCompact` — Restore context after compaction

**Key tools:**
- `qwick_memory_save` — Save a memory (all types)
- `qwick_memory_search` — Semantic search
- `qwick_memory_context` — Load recent context (summary first)
- `qwick_memory_session_summary` — Save structured session summary (with rotation, keeps 3)

This replaces engram. Disable engram when qwick-memory is active.

## Design Spec

Full architecture and design decisions: `docs/superpowers/specs/2026-03-20-qwick-rag-design.md`
