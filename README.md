# qwick-memory

Centralized RAG memory for multiple repositories. A Python CLI and Claude Code plugin that stores decisions, bugs, conventions, and discoveries as searchable, vector-embedded memories shared across your team via git.

Memories are plain markdown files with YAML frontmatter — git handles sharing and merging. The vector index is a local cache rebuilt from those files using LanceDB and local embeddings (fastembed).

## Quick Start

```bash
# Install from source
uv pip install -e ".[dev]"

# Save a memory (auto-detects repo and author from git)
qwick-memory save "We use PostgreSQL for transactional services" --type decision --tags db,postgres

# Search across all memories
qwick-memory search "what database do we use"

# List all memories
qwick-memory list

# Rebuild vector index (after git pull)
qwick-memory index

# Check system health
qwick-memory doctor
```

## How It Works

```
Developer saves a memory
  -> Markdown file written to ~/.qwick-memory/memories/{repo}/{id}.md
  -> Embedded locally via fastembed (all-MiniLM-L6-v2)
  -> Indexed in local LanceDB (.vectordb/, gitignored)
  -> Auto-committed and pushed to origin/memories branch

Team shares via git
  -> Memories auto-sync to an orphan "memories" branch on the same remote
  -> Each developer's index rebuilds automatically
  -> No remote database needed
```

## Claude Code Plugin

Install as a Claude Code plugin for LLM-powered memory:

```bash
# Add the marketplace
claude plugin add --marketplace SidegigLLC/qwick-memory

# Or install directly from the repo
claude mcp add qwick-memory -- uv run --directory /path/to/qwick-memory python -m qwick_memory.server
```

This gives Claude Code 7 MCP tools: `qwick_memory_save`, `qwick_memory_search`, `qwick_memory_list`, `qwick_memory_delete`, `qwick_memory_index`, `qwick_memory_context`, `qwick_memory_session_summary`.

## Memory Types

| Type | Use for |
|------|---------|
| `decision` | Architecture, tool, or workflow choices |
| `bug` | Bug root causes and fixes |
| `convention` | Coding standards, naming patterns |
| `discovery` | Non-obvious findings, gotchas |
| `pattern` | Established approaches |
| `preference` | User or team preferences |
| `note` | General knowledge |

## Project Structure

```
qwick-memory/
├── .vectordb/             # Local LanceDB index (gitignored)
├── src/qwick_memory/         # Python package
│   ├── cli.py             # Typer CLI
│   ├── server.py          # MCP server (FastMCP)
│   ├── memory.py          # Memory model + frontmatter I/O
│   ├── index.py           # LanceDB indexing
│   ├── search.py          # Hybrid search pipeline
│   ├── config.py          # Shared path/context helpers
│   ├── git_utils.py       # Git auto-detection + sync
│   └── errors.py          # Error types
├── tests/                 # Test suite (49 tests)
├── .claude-plugin/        # Claude Code plugin manifest
├── skills/memory/         # Memory protocol for Claude
└── docs/superpowers/      # Design spec and implementation plan
```

## Development

```bash
uv pip install -e ".[dev]"    # Install with dev deps
pytest                         # Unit + integration tests
./scripts/e2e-test.sh          # Real CLI end-to-end test (28 checks)
./scripts/e2e-test.sh --build  # Install from source + run e2e
ruff check src/ tests/         # Lint
ruff format src/ tests/        # Format (2-space indent)
pyright src/                   # Type check
```

## Design

See [docs/superpowers/specs/2026-03-20-qwick-rag-design.md](docs/superpowers/specs/2026-03-20-qwick-rag-design.md) for the full architecture and design decisions.
