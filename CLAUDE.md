# CLAUDE.md

## Project Overview

`qwick-memory` is a Rust CLI that fuses engram-style developer memory, grepai-style
semantic code search, and ast-grep AST patterns into a single binary, knit
together by a two-layer property graph (memory + code). It is a **standalone
agentic-RAG toolbox** invoked from the command line — not a Claude Code MCP
plugin or in-process LLM. Everything runs locally: markdown is the source of
truth, vectors live in LanceDB, structural links live in kuzu.

## Architecture

- **Source of truth:** markdown files with YAML frontmatter at
  `~/.qwick-memory/memories/{id}-{slug}.md` (override with `QWICK_MEMORY_DATA_DIR`).
- **Vector indices:** `lancedb 0.29` embedded, two tables —
  `memory_chunks` (memory bodies) and `code_chunks` (symbol snippets).
- **Property graph:** `kuzu 0.7` with a memory layer (`Memory`, `Repo`,
  `Author`, `Tag`) and a code layer (`File`, `Symbol`) plus cross-layer
  edges (`ReferencesFile`, `ReferencesSymbol`, `RelatesTo`, `Supersedes`,
  `ConflictsWith`).
- **AST extraction:** `ast-grep-core 0.38` + `ast-grep-language 0.38` for
  symbol enumeration and user-supplied `qwick-memory ast` patterns.
- **Embedders:** `fastembed 4` — `nomic-embed-text-v1.5-Q` for memories,
  `jina-embeddings-v2-base-code-Q` for code (ONNX, local, no API calls).
- **Stats / indexing markers:** `rusqlite 0.32` (bundled SQLite).
- **Output:** TTY via `owo-colors`, JSON via `serde_json`. Exit codes follow
  `sysexits.h`.
- **No in-process LLM.** All ranking is deterministic (vector similarity,
  hybrid + corrective fallback, graph walks).

## Key Commands

```bash
cargo install --path .          # build + install the binary locally
just check                      # umbrella gate (alias of scripts/check-all.sh)
just test                       # cargo nextest run --all-features
just qa                         # check-all + cargo-deny + dup-check
just e2e                        # real-binary end-to-end harness
bash scripts/check-all.sh       # the umbrella gate (CI parity)
cargo nextest run --all-features
qwick-memory doctor                    # runtime health check
```

## Binding Rules (apply to every contribution)

These are reproduced verbatim from the implementation plan header and are
enforced by `scripts/check-all.sh`. Every PR must satisfy all five.

1. **No duplication / redundancy.** Shared logic is extracted into a helper.
   Enforced by `scripts/dup-check.sh` and reviewer scrutiny.
2. **Very modular modules.** Each `src/<module>/` directory contains
   narrow, single-purpose files. Files that change together live together.
3. **≤500 lines per file in `src/` or `scripts/`.** Enforced by
   `scripts/module-size-check.sh`.
4. **Zero errors, zero warnings.** No `#[allow(...)]` overrides, no
   `// clippy::allow`, no `.unwrap()` outside `tests/`, no `expect(` without
   a message, no `println!` / `eprintln!` / `todo!()` / `unimplemented!()` /
   `panic!` in `src/`, no `unsafe { … }` without an adjacent `// SAFETY:`
   comment within 3 lines above. Enforced by `scripts/no-bypass-check.sh`.
5. **Tests strictly in `tests/` mirroring `src/` 1:1.** No
   `#[cfg(test)] mod tests { … }` block ever appears inside any file in
   `src/`. Items needing tests are exposed via `pub(crate)`. Each
   `tests/<module>.rs` is a thin test binary that declares submodules in
   `tests/<module>/`. Enforced by `scripts/test-placement-check.sh` and
   `scripts/tests-mirror-check.sh`.

## Code Style

- `rustfmt` defaults — **4-space indent**, 100-column line length
  (`rustfmt.toml`).
- `cargo clippy --all-targets --all-features -- -D warnings`.
- Doc comments (`///`) on every public item.
- `Result<T>` alias from `crate::prelude::*`; errors flow through the
  `Error` enum in `src/errors.rs`.
- Use `tracing` for diagnostics, never `println!` / `eprintln!`.

## Module Map

| Module | Responsibility |
|--------|---------------|
| `cli/` | clap subcommand entry points + the top-level dispatcher in `mod.rs` |
| `memory/` | markdown I/O, `Frontmatter`, slug, id (8-hex SHA-256), atomic save / load / soft-delete / list |
| `index/` | `Embedder` (fastembed wrapper), `MemoryIndex` + `CodeIndex` LanceDB tables, schema |
| `graph/` | kuzu schema, `Graph` upserts, `query` (Cypher helpers), `cross_link` reference extraction |
| `retrieval/` | adaptive `router`, `hybrid` search, `corrective` fallback, `rank` blending, `bundle` for `qwick-memory context` |
| `ast/` | `extractor` (symbol enumeration via tree-sitter through ast-grep), `pattern` (user-facing `qwick-memory ast`), per-language wiring |
| `stats/` | rusqlite stats store + feedback table |
| `config/` | layered config (defaults → file → env) and `Paths` (data-dir layout) |
| `output/` | TTY (`owo-colors`) and JSON (`serde_json`) emitters, shared between subcommands |
| `prune/` | orphan / low-value / stale-code detection plus soft-delete & gc |
| `git_utils.rs` | repo + author auto-detection, git-hook installation helpers |
| `errors.rs` | `thiserror`-derived `Error` enum and `Result<T>` alias |
| `prelude.rs` | crate-internal prelude (`Error`, `Result`, common imports) |
| `lib.rs` / `main.rs` | library surface + binary entry that parses `Cli` and calls `cli::run` |

## Environment Variables

Values are layered: defaults (`Config::defaults`) → optional config file →
environment (`Config::with_env`, in `src/config/file.rs`).

| Variable | Purpose | Default |
|----------|---------|---------|
| `QWICK_MEMORY_DATA_DIR` | Root data directory (memories + lancedb + kuzu + sqlite) | `~/.qwick-memory` |
| `QWICK_MEMORY_INDEXING_AUTO_REINDEX` | `lazy` \| `hook` \| `off` — controls automatic code-index refresh | `lazy` |
| `QWICK_MEMORY_RETRIEVAL_TOP_K` | Number of results returned by the hybrid router | `12` |
| `QWICK_MEMORY_RETRIEVAL_MEMORY_THRESHOLD` | Minimum cosine similarity for the memory table | `0.55` |
| `QWICK_MEMORY_RETRIEVAL_CODE_THRESHOLD` | Minimum cosine similarity for the code table | `0.50` |
| `QWICK_MEMORY_GIT_AUTO_SYNC` | `true`/`1` to enable best-effort git commit + push after a save | `false` |

CLI flags `--data-dir` and `--json` are global and can appear before or
after the subcommand.

## Memory Data Model

Frontmatter schema v1, defined by `src/memory/frontmatter.rs::Frontmatter`:

```yaml
---
id: a1b2c3d4                  # 8-hex prefix of SHA-256(body.trim_end())
kind: decision                # decision|bug|convention|discovery|pattern|note
repo: qwick-backend           # single repo string (not a list)
tags: [database, postgres]
author: falconiere
created: 2026-05-17T14:30:00Z
quality: 4                    # 1..=5, default 3
schema: 1
content_hash: <64-hex SHA-256 of body.trim_end()>
references:
  symbols: []                 # qualified symbol ids: <repo>:<path>:<name>
  files:   []                 # qualified file paths:  <repo>:<path>
relations:
  supersedes:     []          # memory ids this one replaces
  conflicts_with: []          # memory ids this one contradicts
  derived_from:   []          # memory ids this one builds on
---

Markdown body lives here.
```

## Save Flow (spec §8, current implementation)

`qwick-memory save` runs:

1. Parse args, resolve repo/author defaults, build `Frontmatter` with
   `schema: 1` and `content_hash = sha256(body.trim_end())`.
2. Atomic stage: write `memories/.{id}.tmp`, then `fs::rename` to
   `memories/{id}-{slug}.md`. On any failure between stage and rename, the
   tmp file is removed.
3. Best-effort graph upsert (`src/cli/save.rs::upsert_graph`):
   - `Graph::upsert_memory` — `Memory` node + `InRepo`, `AuthoredBy`,
     `Tagged` edges.
   - `cross_link::extract_refs` walks the body for backtick-fenced
     `<repo>:<path>` / `<repo>:<path>:<symbol>` mentions and emits
     `ReferencesFile` / `ReferencesSymbol` edges (silently no-op when the
     File/Symbol nodes do not yet exist — `qwick-memory index-code` fills them in
     later).
   Graph failures are logged via `tracing::warn!` and swallowed; markdown
   remains the source of truth.

**v1.x gaps** (tracked in `README.md` "Known v1.1 gaps"):

- The memory body is **not yet** embedded into `lancedb.memory_chunks` from
  the save path itself; rebuild via `qwick-memory index-code` or a future
  dedicated `qwick-memory index` command.
- `RelatesTo` neighbor discovery (spec §8 step 6) is deferred.
- Git auto-sync from `git_utils` runs only when `QWICK_MEMORY_GIT_AUTO_SYNC` is
  enabled.

## Testing

- Runner: `cargo nextest run --all-features` (alias `just test`).
- `tests/` mirrors `src/` 1:1. Each top-level test binary
  (`tests/<module>.rs`) is a thin shim that declares submodules in
  `tests/<module>/`.
- `tests/common/` carries shared fixtures (temp data-dir builders, gold
  memory samples).
- CLI integration tests use `assert_cmd` against the real `qwick-memory` binary.
- Snapshot tests use `insta` (`tests/snapshots/`).
- Property tests use `proptest`.
- `.config/nextest.toml` serializes the `embedder` test group
  (`embedder|memory_index|code_index`) to `max-threads = 1` so the fastembed
  model download cannot race.

## Quality Gates

`bash scripts/check-all.sh` is the umbrella gate. It runs, in order:

```
scripts/fmt-check.sh             # cargo fmt --check
scripts/type-check.sh            # cargo check --all-targets --all-features
scripts/lint-check.sh            # cargo clippy --all-targets --all-features -- -D warnings
scripts/test-placement-check.sh  # no #[cfg(test)] mod tests in src/
scripts/no-bypass-check.sh       # no allow/unwrap/println!/unsafe-without-SAFETY/etc.
scripts/module-size-check.sh     # no file > 500 lines in src/ or scripts/
scripts/tests-mirror-check.sh    # every src/ file has a mirror in tests/
scripts/typos-check.sh           # typos
```

Additional gates wired into `just qa`: `scripts/deny-check.sh`
(`cargo deny check`) and `scripts/dup-check.sh`. `scripts/test-run.sh`
runs the nextest suite. A task is not "done" until `scripts/check-all.sh`
exits 0.

## Distribution

- `cargo install qwick-memory` (source, from crates.io once published).
- `brew install SidegigLLC/tap/qwick-memory` (Homebrew tap
  `SidegigLLC/homebrew-tap`, published by `cargo-dist`).
- Prebuilt tarballs for `aarch64-apple-darwin`, `x86_64-apple-darwin`,
  `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu` attached to
  [GitHub Releases](https://github.com/SidegigLLC/qwick-memory/releases).

`cargo-dist` is configured in `[package.metadata.dist]` in `Cargo.toml`.
PRs get a dry-run plan; only `v*` tags publish artifacts.

## Claude Code Hooks

`.claude/hooks/` is adapted from `qwick-business-app` and delegates rule
logic to the same gate scripts.

- **PreTool hooks** (`pre-tools/modules/`):
  - `bash-commands.sh` blocks `npm`/`bun`/`yarn`/`pnpm`/`pip`/`uv`/`poetry`
    (this is a Rust project), destructive commands
    (`rm -rf`, `git push --force`, `git reset --hard`, `git checkout .`,
    `chmod -R 777`), bypass flags (`--no-verify`, `--no-gpg-sign`), and
    direct `rustfmt` / `cargo fmt` / `cargo clippy` invocation outside
    `scripts/` or `just`.
  - `code-edit-rules.sh` rejects edits to `src/*.rs` that introduce
    forbidden patterns: `#[allow(...)]`, `// clippy::allow`,
    `#[cfg(test)] mod tests`, `.unwrap()`, empty `.expect()`,
    `println!`/`eprintln!`, `todo!()`/`unimplemented!()`, `panic!()`, or
    `unsafe { … }` without a nearby `// SAFETY:` comment. Mirrors
    `scripts/no-bypass-check.sh`.
  - `protected-files.sh` guards generated artifacts and config the agent
    must not edit casually.
- **PostTool hooks** (`post-tools/modules/`):
  - `auto-format.sh` re-runs `rustfmt` on touched files.
  - `auto-lint.sh` runs clippy on the affected crate.
  - `gate-status.sh` records which gates are currently green for the
    session.
- **Stop hook** (`session-end.sh`) runs `fmt-check`,
  `test-placement-check`, `no-bypass-check`, and `module-size-check` at
  end-of-conversation so regressions surface immediately.

## Spec / Plan References

- Design spec:
  `docs/superpowers/specs/2026-05-17-qwick-rust-agentic-rag-design.md`
- Implementation plan:
  `docs/superpowers/plans/2026-05-17-qwick-rust-agentic-rag-plan.md`

User-facing docs live in `docs/architecture.md` and
`docs/cli-reference.md`; the README links to both.
