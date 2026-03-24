# qwick-memory

Centralized RAG memory for multiple repositories. A Python CLI and Claude Code plugin that stores decisions, bugs, conventions, and discoveries as searchable, vector-embedded memories shared across your team via git.

Memories are plain markdown files with YAML frontmatter — git handles sharing and merging. The vector index is a local cache rebuilt from those files using LanceDB and local embeddings (fastembed, no API calls).

## Quick Start

```bash
# Install globally (puts qwick-memory + qwick-memory-server on PATH)
uv tool install -e ".[dev]"

# Save a memory (auto-detects repo and author from git)
qwick-memory save "We use PostgreSQL for transactional services" --type decision --tags db,postgres

# Search across all memories
qwick-memory search "what database do we use"

# List all memories for a repo
qwick-memory list --repo my-project

# Rebuild vector index (e.g. after git pull)
qwick-memory index

# Check system health
qwick-memory doctor
```

## How It Works

```
Developer saves a memory
  → Markdown file written to ~/.qwick-memory/memories/{id}.md
  → Embedded locally via fastembed (nomic-embed-text-v1.5-Q, 768d, 8K tokens)
  → Indexed in local LanceDB (.vectordb/, gitignored)
  → Auto-committed and pushed to origin/memories branch

Team shares via git
  → Memories auto-sync to an orphan "memories" branch on a configured remote
  → Each developer's vector index rebuilds automatically
  → No remote database needed
```

### Save Flow (Atomic)

1. Generate ID (SHA-256 of content, 12 hex chars)
2. Write markdown to temp file `memories/.{id}.tmp`
3. Embed content via fastembed
4. Upsert into LanceDB
5. Atomic rename temp → final `memories/{id}.md`
6. `git_sync`: add + commit + push (best-effort, never fails the save)
7. On failure: delete temp file, report error

## Claude Code Plugin

qwick-memory integrates with Claude Code as a plugin, giving Claude 7 MCP tools for automatic memory management. When active, Claude proactively saves decisions, bugs, conventions, and discoveries — and searches memory before answering questions about prior work.

### Install via Marketplace (Recommended)

```bash
# Add the marketplace (one-time)
claude plugin marketplace add SidegigLLC/qwick-memory

# Install at user scope (available in all projects)
claude plugin install qwick-memory

# Or install at project scope (only available in this project)
claude plugin install qwick-memory --scope project
```

**Scope options:**
- `--scope user` (default) — plugin is available in all projects for the current user
- `--scope project` — plugin is installed into `.claude/plugins/` in the current project directory (committed to version control, shared with the team)
- `--scope local` — plugin is installed locally for the current project but not committed to version control

### Install Manually (MCP Server)

If you prefer not to use the plugin marketplace:

```bash
# Prerequisite: install globally so the server binary is on PATH
uv tool install -e /path/to/qwick-memory

# Add the MCP server to Claude Code
claude mcp add qwick-memory -- qwick-memory-server
```

### What the Plugin Provides

**7 MCP tools:**

| Tool | Purpose |
|------|---------|
| `qwick_memory_save` | Save a memory (decision, bug, convention, etc.) |
| `qwick_memory_search` | Semantic vector search with metadata filtering |
| `qwick_memory_list` | List memories from disk with optional filters |
| `qwick_memory_delete` | Delete a memory by ID |
| `qwick_memory_index` | Build or rebuild the vector index |
| `qwick_memory_context` | Load recent context (session summary + memories) |
| `qwick_memory_session_summary` | Save a structured session summary |

**3 lifecycle hooks:**

| Hook | Script | Purpose |
|------|--------|---------|
| `SessionStart` | `session-start.sh` | Auto-migrate, auto-index, load context |
| `PreCompact` | `pre-compact.sh` | Remind to save session summary |
| `PostCompact` | `post-compact.sh` | Restore context after compaction |

**Memory protocol:** Claude follows a mandatory SEARCH → SAVE → SUMMARIZE decision tree on every message, so knowledge is never lost between sessions.

## CLI Reference

### `qwick-memory save [CONTENT]`

Save a new memory. Opens `$EDITOR` if content is omitted.

```
Options:
  -t, --type TEXT   Memory type (default: note)
  -r, --repo TEXT   Comma-separated repos (auto-detected if omitted)
  --tags TEXT       Comma-separated tags
  -v, --verbose     Enable verbose logging
```

### `qwick-memory search QUERY`

Search memories by semantic similarity.

```
Options:
  -r, --repo TEXT   Filter by repo
  -t, --type TEXT   Filter by type
  --tag TEXT        Filter by tag
  -n, --limit INT   Max results (default: 10)
  -v, --verbose     Enable verbose logging
```

### `qwick-memory list`

List memories from disk (not the index).

```
Options:
  -r, --repo TEXT   Filter by repo
  -t, --type TEXT   Filter by type
  --tags TEXT       Filter by tags (comma-separated)
  -v, --verbose     Enable verbose logging
```

### `qwick-memory delete ID`

Delete a memory by ID. Removes from disk and vector index.

### `qwick-memory index`

Build or rebuild the vector index. Incremental by default.

```
Options:
  -f, --force   Force full rebuild
  -v, --verbose Enable verbose logging
```

### `qwick-memory migrate`

Auto-migrate memories: flatten nested directories, rebuild index if the embedding model changed. Safe to run repeatedly — called automatically by the `SessionStart` hook.

### `qwick-memory context`

Show recent memories for context restoration. Displays the latest session summary followed by recent memories.

```
Options:
  -r, --repo TEXT   Filter by repo
  -n, --limit INT   Max non-summary memories (default: 10)
  -v, --verbose     Enable verbose logging
```

### `qwick-memory doctor`

Health check: validates memory files, index consistency, model version, and git context.

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
| `session-summary` | Auto-generated session summaries (rotated, keeps 3) |

## Memory Data Model

Each memory is a markdown file with YAML frontmatter:

```yaml
---
id: a1b2c3d4e5f6          # SHA-256 of content, 12 hex chars
repo: [qwick-backend]     # List of repo names
type: decision             # One of the memory types above
tags: [database, postgres] # Tags for filtering
author: falconiere         # Auto-detected from git config
created: 2026-03-20T14:30:00+00:00
content_hash: a1b2c3d4e5f6  # For incremental indexing
---

The actual memory content goes here as markdown body.
```

## Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `QWICK_MEMORY_DIR` | `~/.qwick-memory/` | Root directory for memories and vectordb |
| `QWICK_MEMORY_REPO` | Auto-detected from git remote | Override repo name |
| `QWICK_MEMORY_AUTHOR` | Auto-detected from git config | Override author name |
| `QWICK_MEMORY_REMOTE` | Not set (local only) | Git remote URL for memory sync (`""` to disable) |

### Team Sharing via Git

To share memories across machines or team members, set `QWICK_MEMORY_REMOTE` to a git remote URL:

```bash
export QWICK_MEMORY_REMOTE="git@github.com:yourorg/team-memories.git"
```

Memories are stored on an orphan `memories` branch. On first save, qwick-memory will:
1. Initialize a git repo in `~/.qwick-memory/`
2. Configure the remote
3. Pull existing memories (if any)
4. Auto-commit and push on every save/delete

Without `QWICK_MEMORY_REMOTE`, memories stay local (still git-tracked for history, but not pushed).

## Project Structure

```
qwick-memory/
├── src/qwick_memory/      # Python package
│   ├── cli.py             # Typer CLI (8 commands)
│   ├── server.py          # MCP server (FastMCP, 7 tools)
│   ├── memory.py          # Memory dataclass, markdown I/O, ID generation
│   ├── index.py           # LanceDB: embed, upsert, delete, incremental rebuild
│   ├── search.py          # Hybrid search (vector + BM25 fallback)
│   ├── config.py          # Shared path/context helpers
│   ├── git_utils.py       # Git auto-detection + sync (orphan branch)
│   └── errors.py          # QwickRagError hierarchy
├── scripts/               # Shell scripts (self-locating, work from any CWD)
│   ├── mcp-server.sh      # MCP server launcher
│   ├── session-start.sh   # Auto-index + context on session start
│   ├── pre-compact.sh     # Reminder before context compaction
│   ├── post-compact.sh    # Restore context after compaction
│   └── e2e-test.sh        # End-to-end CLI test (28 checks)
├── hooks/hooks.json       # Claude Code lifecycle hooks
├── skills/memory/         # Memory protocol skill for Claude
├── .claude-plugin/        # Claude Code plugin manifest
├── tests/                 # Test suite (49 tests)
└── docs/superpowers/      # Design specs and implementation plans
```

## Development

```bash
uv tool install -e ".[dev]"     # Install globally with dev deps
pytest                           # Unit + integration tests (49 tests)
./scripts/e2e-test.sh            # Real CLI end-to-end test (28 checks)
./scripts/e2e-test.sh --build    # Install from source + run e2e
ruff check src/ tests/           # Lint
ruff format src/ tests/          # Format (2-space indent)
pyright src/                     # Type check
```

**First test run** downloads the embedding model (~130MB, cached at `~/.cache/fastembed/`).

## Design

See [docs/superpowers/specs/2026-03-20-qwick-rag-design.md](docs/superpowers/specs/2026-03-20-qwick-rag-design.md) for the full architecture and design decisions.

## License

MIT
