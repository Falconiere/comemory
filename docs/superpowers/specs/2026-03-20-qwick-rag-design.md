# qwick-memory: Centralized RAG Memory for Multiple Repositories

**Date:** 2026-03-20
**Status:** Approved

## Overview

qwick-memory is a Python CLI tool and Claude Code plugin that provides centralized, persistent memory for multiple repositories. It stores decisions, bugs, conventions, patterns, and discoveries as vector-embedded observations, enabling semantic search across all project knowledge.

Memories are plain markdown files committed to git (the source of truth). The vector index is a local cache rebuilt from those files. Git handles sharing and merging across the team.

## Architecture

```
┌─────────────────────────────────────────────────┐
│              qwick-memory (CLI + MCP)               │
│  (Claude Code plugin + pip-installable CLI)      │
├─────────────────────────────────────────────────┤
│  MCP Tools: rag_save, rag_search, rag_list, ... │
│  CLI: qwick-memory save, search, list, index, ...  │
├──────────┬──────────────────┬───────────────────┤
│ Embedding│   Memory Manager │   Git Utils       │
│ (sentence│  (CRUD + markdown│  (repo detection, │
│-transform│   frontmatter)   │   author)         │
│  ers)    │                  │                   │
├──────────┴──────────────────┴───────────────────┤
│              LanceDB (embedded)                  │
│  File-based, ACID, versioned, memory-mapped      │
│  .vectordb/ (gitignored)                         │
└─────────────────────────────────────────────────┘
```

### Key Architectural Decisions

- **Markdown source of truth:** Memories are human-readable markdown files with YAML frontmatter, committed to git. The vector index is a derived cache that can be rebuilt at any time.
- **LanceDB embedded:** File-based vector DB with Rust core (PyO3), ACID transactions, MVCC versioning, memory-mapped storage (~4MB idle). No server, no Docker, no root access required. No point limit (tested at 700M+ vectors in production).
- **Local embeddings:** ONNX-based embedding via `fastembed` (`all-MiniLM-L6-v2`, 384 dimensions, ~30MB model). Runs on CPU, no API keys, no cost per call. Uses ONNX Runtime instead of PyTorch to keep the install lightweight (~50MB vs ~2GB for torch).
- **Hybrid search:** LanceDB's built-in BM25 (via Tantivy) + vector similarity, merged with RRF reranking.
- **Claude Code plugin:** Distributed via the Claude Code marketplace. Exposes MCP tools for LLM agents + CLI for developers.
- **Git-based sharing:** Team members share memories via git push/pull. No remote database, no binary merge conflicts.

### Why These Choices

| Decision | Alternatives considered | Why this one |
|----------|------------------------|--------------|
| LanceDB over Qdrant local | Qdrant local mode is a Python compatibility shim (NumPy brute-force, 20K point limit, hundreds of MB RAM). LanceDB is a real embedded DB with Rust core, ACID, versioning, no limits, 4MB idle. |
| LanceDB over Qdrant server | Requires Docker, contradicts "no server" goal. |
| LanceDB over ChromaDB | ChromaDB degrades under concurrent load, single-node only, auth removed in 1.0. |
| LanceDB over pgvector | Overkill for a memory CLI tool. Heavy setup and tuning. |
| Markdown files over binary DB in git | Binary files can't be merged in git. Markdown merges cleanly. The vector index is a derived artifact. |
| fastembed over fastembed | fastembed pulls in PyTorch (~2GB). fastembed uses ONNX Runtime (~50MB), same model quality, much lighter install for a CLI tool. fastembed works independently of Qdrant. |
| Local embeddings over OpenAI API | Zero cost, no API keys, works offline, fast enough for this use case. |

## Memory Data Model

Each memory is a markdown file with YAML frontmatter stored at `memories/{repo}/{id}.md`:

```markdown
---
id: a1b2c3d4
repo: qwick-backend
type: decision
tags: [database, postgres, migrations]
author: falconiere
created: 2026-03-20T14:30:00Z
---

We chose PostgreSQL over MongoDB for the order service because we need
strong transactional guarantees for payment flows. The team considered
MongoDB for flexibility but the consistency requirements won out.
```

### Fields

| Field | Source | Description |
|-------|--------|-------------|
| `id` | Generated | SHA-256 of content, truncated to 12 hex characters (48 bits, collision-safe up to ~16M memories). Used as filename and LanceDB row ID. |
| `repo` | Auto-detected | From `git remote get-url origin` in the caller's working directory. Falls back to directory name. |
| `type` | User-provided | One of: `decision`, `bug`, `convention`, `discovery`, `pattern`, `preference`, `note`. |
| `tags` | User-provided | Freeform list for filtering. |
| `author` | Auto-detected | From `git config user.name`. |
| `created` | Generated | ISO 8601 timestamp. |

Content hashing provides natural deduplication: saving the exact same memory twice produces the same file.

Memories are append-only by design. To update a memory, delete the old one and save a new version. The content hash changes, producing a new file. This keeps the git history clean and avoids partial-update edge cases.

### Embedding Limits

`all-MiniLM-L6-v2` has a 256-token input limit. Content beyond this is silently truncated by the tokenizer, meaning embeddings only capture the beginning of the text. This is acceptable for memories, which should be concise (a few sentences to a short paragraph). The CLI should warn if content exceeds 256 tokens and suggest splitting into multiple memories.

## CLI Interface

Installed via pip, used from any directory:

```bash
pip install qwick-memory
```

### Commands

```bash
# Save a memory
qwick-memory save "We use PostgreSQL for transactional services"
qwick-memory save --type decision --tags db,postgres "We chose PG over Mongo"
qwick-memory save   # opens $EDITOR for longer memories

# Semantic search
qwick-memory search "what database do we use"
qwick-memory search "auth pattern" --repo qwick-backend --type convention
qwick-memory search "logging" --limit 5

# List memories
qwick-memory list --repo qwick-backend
qwick-memory list --type bug --tags payments

# Delete a memory
qwick-memory delete a1b2c3d4

# Rebuild vector index from markdown files
qwick-memory index
qwick-memory index --force   # full rebuild (new embedding model, corruption recovery)

# Diagnostics
qwick-memory doctor
```

### Behaviors

- `repo` and `author` are auto-detected from the current git context.
- `save` writes a markdown file + upserts the embedding into local LanceDB.
- `search` runs hybrid search (vector + BM25) with optional metadata filters.
- `index` is incremental: content-hashes each file, only re-embeds new/changed ones. Runs `table.optimize()` to compact old LanceDB versions.
- `index --force` drops the LanceDB table and rebuilds from scratch. Used when changing embedding models or recovering from corruption.
- `doctor` checks `.vectordb/` health, `memories/` readability, index consistency, git context, embedding model availability, embedding model version consistency, and disk space.

## MCP Server Interface (Claude Code Plugin)

### MCP Tools

| Tool | Description |
|------|-------------|
| `rag_save` | Save a memory (content, type, tags). Auto-detects repo + author. |
| `rag_search` | Semantic + keyword hybrid search with optional repo/type/tag filters. |
| `rag_list` | List memories with filters. |
| `rag_delete` | Delete a memory by ID. |
| `rag_index` | Rebuild vector index from markdown files. |
| `rag_context` | Get recent/relevant memories for the current repo (top 20 by recency, with semantic boost). Useful for session start context loading. |

### Plugin Structure

```
qwick-memory/
├── .claude-plugin/
│   ├── plugin.json              # Plugin manifest
│   └── marketplace.json         # Marketplace distribution
├── .mcp.json                    # MCP server launch config
├── hooks/
│   └── hooks.json               # SessionStart lifecycle hook
├── scripts/
│   └── session-start.sh         # Auto-index on session start
├── skills/
│   └── memory/
│       └── SKILL.md             # Memory protocol for Claude
├── memories/                    # Git-tracked, shared with team
│   └── {repo}/
│       └── {id}.md
├── src/
│   └── qwick_rag/
│       ├── __init__.py
│       ├── __main__.py          # CLI entry point (typer)
│       ├── server.py            # MCP server (FastMCP)
│       ├── memory.py            # Memory model (parse/write markdown)
│       ├── index.py             # LanceDB indexing
│       ├── search.py            # Query pipeline (hybrid search)
│       └── git_utils.py         # Auto-detect repo, author
├── tests/
├── pyproject.toml
├── .gitignore                   # includes .vectordb/
├── .vectordb/                   # local only, gitignored
└── docs/
```

### Plugin Configuration Files

**`.claude-plugin/plugin.json`:**

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

**`.mcp.json`:**

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

### Distribution

```bash
/plugin marketplace add SidegigLLC/qwick-memory
/plugin install qwick-memory@qwick-memory
```

## Indexing & Search Pipeline

### Save Flow

```
1. Generate ID (SHA-256 of content → short hash)
2. Auto-detect repo (git remote) + author (git user)
3. Write markdown to temp file: memories/{repo}/.tmp_{id}.md
4. Embed content via fastembed (all-MiniLM-L6-v2)
5. Upsert into LanceDB with metadata + vector
6. Atomic rename temp file → memories/{repo}/{id}.md
── If any step fails: delete temp file, report error
```

### Search Flow

```
1. Embed query via fastembed
2. Apply metadata filters (repo, type, tags) via .where() — pushed into retrieval
3. LanceDB hybrid search: vector similarity + BM25 full-text (filtered)
4. RRF reranking merges both result sets
5. Return top results with: score, repo, type, tags, content preview
```

### Index Rebuild Flow

```
1. Scan all memories/**/*.md files
2. Content-hash each file
3. Compare against existing LanceDB rows (stored hash in metadata)
4. Skip unchanged, embed + upsert new/modified, delete orphaned
5. Run table.optimize() to compact old versions
6. Report: "Indexed 12 new, 3 updated, 1 deleted"
```

## Error Handling

### Structured Error Types

```
QwickRagError (base)
├── StorageError          # File system (permissions, disk full, path not found)
├── VectorIndexError       # LanceDB (corrupt DB, embedding failures)
├── GitError              # Git detection (no remote, no user config)
├── MemoryParseError      # Malformed frontmatter, invalid YAML
└── ConfigError           # Missing dependencies
```

Each error carries: error code, human-readable message, suggested fix, and context (file path, operation attempted).

### Recovery Strategies

| Scenario | Detection | Recovery | Fallback |
|----------|-----------|----------|----------|
| `.vectordb/` corrupted | LanceDB throws on connect | Roll back via `table.checkout(version)` | `qwick-memory index --force` to rebuild from scratch |
| Markdown file malformed | YAML parse error | Skip file, log warning with path + line | Continue indexing remaining files |
| Embedding model download fails | Network/timeout | Retry 3x with exponential backoff | Fail with clear message |
| Disk full on save | OSError on file write | Fail before writing to LanceDB (atomic) | Report available disk space |
| Git remote not found | `git remote` returns empty | Warn, use directory name as repo | Store `repo_source: "directory_name"` in metadata |
| Crash during write | Process killed | ACID atomic writes — partial writes never committed | Next `index` cleans up orphans |
| Memory file deleted outside tool | In LanceDB but not on disk | `qwick-memory index` detects orphans, removes from LanceDB | Automatic on next index |
| LanceDB version bloat | Storage grows over time | `table.optimize()` runs on every `index` | `index --force` for clean slate |

### Atomic Save Guarantee

`rag_save` is all-or-nothing:

1. Write markdown to temp file (`memories/{repo}/.tmp_{id}.md`)
2. Embed content
3. Upsert into LanceDB
4. Atomic rename temp file to final path (POSIX atomic)
5. If any step fails: delete temp file, skip LanceDB upsert, report error

If the process crashes between step 3 and 4, `qwick-memory index` detects the orphan in LanceDB (no matching file) and cleans it up.

### Logging

- Structured logging via Python `logging` module
- `DEBUG`: embedding timings, file hashes, LanceDB operations
- `INFO`: saves, searches, index operations
- `WARNING`: skipped files, fallback behaviors, git detection failures
- `ERROR`: operation failures with context and suggested fix
- MCP server → stderr (required by MCP protocol)
- CLI → stderr for errors/warnings, stdout for results

### Diagnostics

`qwick-memory doctor` checks:
- `.vectordb/` health (can LanceDB connect and read?)
- `memories/` readability (any malformed files?)
- Index consistency (orphans? missing entries?)
- Git context (remote? user config?)
- Embedding model availability and version consistency (`.vectordb/meta.json` stores model name; warns on mismatch with current config)
- Disk space

## Dependencies

| Package | Purpose |
|---------|---------|
| `lancedb` | Embedded vector DB (Rust core, ACID, versioned) |
| `fastembed` | Local embeddings via ONNX Runtime (all-MiniLM-L6-v2, ~50MB) |
| `python-frontmatter` | Parse/write YAML frontmatter in markdown |
| `typer` | CLI framework |
| `rich` | Terminal output formatting |
| `mcp` | MCP server SDK (FastMCP) |

## Team Sharing Workflow

```
Developer A (in qwick-backend repo):
  qwick-memory save --type decision "We use Redis for session caching"
  cd ~/Projects/qwick-memory && git add . && git commit -m "Add Redis caching decision" && git push

Developer B:
  cd ~/Projects/qwick-memory && git pull
  qwick-memory index   # rebuilds vector index with new memories
  qwick-memory search "session caching"   # finds Developer A's memory
```

Git handles all sharing and merging. Each memory is its own file, so conflicts are near-impossible with append-only usage. The vector index is gitignored and rebuilt locally.
