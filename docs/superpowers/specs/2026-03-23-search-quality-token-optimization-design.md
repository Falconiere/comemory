# Search Quality, Token Optimization & Scale Improvements

**Date:** 2026-03-23
**Status:** Approved
**Scope:** qwick-memory plugin — embedding upgrade, token-aware formatting, repo handling, protocol reinforcement, scale

## Problem Statement

Five interconnected issues limit qwick-memory's effectiveness, especially in monorepo contexts like qwick-apps:

1. **Search quality** — `all-MiniLM-L6-v2` (256-token input, 384 dims) silently truncates memories beyond ~800 words, losing semantic context.
2. **Token waste** — Search and context tools dump raw text into Claude's context without structure or relevance prioritization. In monorepos with many memories, this means noise.
3. **Model attention** — Flat result formatting makes it easy for Claude to skim over or miss key details from retrieved memories.
4. **Claude skips saves/searches** — Despite the behavioral protocol, Claude sometimes omits `qwick_memory_save` or `qwick_memory_search` calls.
5. **Repo misattribution** — Auto-detection from git remote returns the monorepo name (e.g., `qwick-apps`) when the memory belongs to a sub-project (e.g., `sidegig-api`, `qwick-mobile`).

## Approach

Phased delivery (C): high-impact foundation first, monorepo intelligence second, advanced retrieval third (only if needed).

---

## Phase 1: Foundation

### 1.1 Embedding Model Upgrade

**Change:** Replace `sentence-transformers/all-MiniLM-L6-v2` with `nomic-ai/nomic-embed-text-v1.5`.

| Property | Old (MiniLM) | New (Nomic) |
|----------|-------------|-------------|
| Dimensions | 384 | 768 |
| Max tokens | 256 (~800 words) | 8192 (~24K words) |
| Model size | ~30MB | ~520MB |
| Embedding speed | Baseline | ~2x slower |

**Implementation details:**

- Update `MODEL_NAME` in `index.py` to `"nomic-ai/nomic-embed-text-v1.5"`.
- Add **query/document prefixes** required by nomic:
  - At embed time (indexing): prepend `"search_document: "` to memory content.
  - At search time: prepend `"search_query: "` to the query string.
- Update `meta.json` write to include model name.
- `doctor` command: detect model mismatch between `meta.json` and current `MODEL_NAME`. Report as error with suggested fix: `qwick-memory index --force`.
- `build()`: when `meta.json` model doesn't match `MODEL_NAME`, auto-trigger force rebuild with a log message.

**Files:** `index.py`, `search.py`

**Breaking change:** Requires `qwick-memory index --force` after upgrade. Dimension change (384 → 768) makes old index incompatible.

### 1.2 Token-Aware Result Formatting

**Change:** Structure search results into relevance tiers with a token budget.

**Relevance tiers:**

| Tier | Score threshold | Content treatment |
|------|----------------|-------------------|
| High | > 0.7 | Full content |
| Moderate | 0.4–0.7 | Truncated to ~200 chars, memory ID for full fetch |
| Low | < 0.4 | One-liner: type, repo, first sentence, ID |

Score thresholds are constants in `server.py`, tunable after observing nomic's score distribution.

**Token budget:**

- New constant `DEFAULT_TOKEN_BUDGET = 4000` (estimated as `len(text) // 4`).
- Search tool: allocate budget top-down by relevance. High-relevance results consume first, then moderate, then low. If budget is exhausted, remaining results are dropped.
- Context tool: session summary gets priority allocation (~1000 tokens), remaining budget fills with recent memories by recency + type diversity.

**Structured markdown output format:**

```markdown
### High Relevance
**[decision] JWT auth middleware** — sidegig-api (tags: auth, jwt)
Full memory content here...

### Moderate Relevance
**[bug] Login timeout on slow connections** — qwick-mobile (tags: auth, mobile)
First 200 chars of content... → ID: abc123def456

### Low Relevance
- [discovery] React Native deep linking — qwick-mobile → ID: def789abc012
```

**Files:** `server.py`

### 1.3 Repo as Required Field

**Change:** Remove auto-detection from save path. `repo` is always required, passed explicitly like tags.

**Save tool (`qwick_memory_save`):**
- `repo` parameter: remove default empty string. If empty/missing → return error: `"Error: repo is required. Specify which repo(s) this memory belongs to (e.g. 'sidegig-api' or 'sidegig-api,sidegig-web')."`
- Remove `get_repo()` call from save flow.
- Tool description updated: `"Comma-separated repo names (e.g. 'qwick-mobile' or 'sidegig-api,sidegig-web'). REQUIRED — always specify."`

**Session summary tool (`qwick_memory_session_summary`):**
- Same change: `repo` required, no fallback.

**Search and context tools:**
- Keep `get_repo()` as optional default filter. These tools benefit from auto-detection as a convenience — filtering, not attribution.

**CLI (`cli.py`):**
- `save` command: make `--repo` required when no `.git` detected (current behavior, no change).

**Flat layout enforcement:**
- `write_memory()`: validate that target path parent is exactly `memories_dir`. Raise `StorageError` if nested.
- `scan_memories()`: keep `glob("*.md")` (already flat). Add `logger.warning` if any subdirectories exist in `memories/`.
- `doctor`: add check "No nested directories in memories/". Warn with suggested cleanup.

**Files:** `server.py`, `memory.py`, `cli.py`

### 1.4 Protocol Reinforcement

**Tool descriptions (strongest signal for Claude):**

- `qwick_memory_save`: Remove "Auto-detected from git remote when available" language. Add: `"REQUIRED: always provide repo. Never omit it."`
- `qwick_memory_search`: Add: `"If you're about to answer from general knowledge, STOP — search first. Memory has project-specific context you don't."`

**Response nudges (sharpened):**

- After save: `"Saved for [sidegig-api]. Tags: auth, jwt."` — Explicit confirmation of what was stored.
- After search with results: `"N results found. Use these to inform your response — do NOT ignore them."`
- After search, no results: `"No results. If you learn something new about this topic, save it before the session ends."`

**SessionStart hook:**
- Add to context output: `"REMINDER: save decisions, bugs, and discoveries to qwick-memory. Always specify repo."`

**Files:** `server.py`, `scripts/session-start.sh`

### 1.5 Scale Guardrails

**FTS index rebuild optimization:**
- Remove FTS rebuild from `upsert()`. Individual saves rely on vector search.
- FTS index rebuilds only during `build()` (full or incremental).
- Reduces per-save overhead as memory count grows.

**Context loading optimization:**
- Context tool auto-filters by current repo (from `get_repo()`) first, then fills remaining budget with cross-repo memories.

**Files:** `index.py`, `server.py`

---

## Phase 2: Monorepo Intelligence (future, if needed)

- Query expansion with current repo/file context
- Sub-project aware search boosting
- Cross-cutting memory surfacing

## Phase 3: Advanced Retrieval (future, if needed)

- Cross-encoder re-ranking for precision
- Memory chunking for long content
- Usage analytics

---

## Testing Strategy

### Embedding model upgrade tests (~4 tests)
- **Model prefix correctness**: verify `"search_query: "` / `"search_document: "` prefixes applied.
- **Long content retrieval**: save 2000+ word memory, search for phrase at word 1500+. Assert found and high-ranked.
- **Model mismatch detection**: build index with 384-dim mock, call `build()` with new model. Assert force rebuild triggered or clear error.
- **Meta.json version tracking**: after rebuild, `meta.json` contains new model name. `doctor` reports healthy.

### Repo-required tests (~3 tests)
- **Save without repo → error**: `qwick_memory_save(content="...", repo="")` with no git context. Assert error.
- **Save with multi-repo**: `repo="sidegig-api,sidegig-web"`. Assert frontmatter has `repo: [sidegig-api, sidegig-web]`.
- **Search cross-repo**: save for two repos, search with and without filter. Assert correct filtering.

### Token-aware formatting tests (~3 tests)
- **Tiered output**: mock 10 results with scores 0.1–0.9. Assert high=full, moderate=truncated, low=one-liner.
- **Token budget enforcement**: set budget to 2000 tokens, feed 10 long memories. Assert output under budget, high-relevance preserved.
- **Context budget**: load context with 20 memories. Assert session summary gets priority, total within budget.

### Flat layout tests (~3 tests)
- **Write to nested path → StorageError**: attempt write to `memories/0.1.0/abc.md`. Assert raises.
- **Doctor detects nested dirs**: create subdirectory in `memories/`. Assert warning.
- **Scan ignores nested**: place `.md` in `memories/subdir/`. Assert not found by `scan_memories`.

### Protocol tests (~2 tests)
- **Save response includes repo confirmation**: assert success message names repos explicitly.
- **Search empty → save hint**: assert no-results message includes nudge.

**Total: ~15 new tests.**

**Not adding:**
- No embedding quality benchmarks (model's responsibility).
- No load tests (premature for < 1000 memories).
- No cross-encoder tests (Phase 3).

---

## Migration Path

1. Update code (all Phase 1 changes).
2. Run `qwick-memory index --force` to rebuild with nomic embeddings.
3. `doctor` verifies healthy state.
4. Existing memories unchanged on disk — only the vector index is rebuilt.

## Dependencies

- `fastembed >= 0.7` (already supports `nomic-ai/nomic-embed-text-v1.5`)
- `lancedb >= 0.30` (already supports 768-dim vectors)
- No new dependencies required.
