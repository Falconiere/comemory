# AGENTS.md

## What this project is

`comemory` is a Rust CLI binary — a local-first agentic-RAG toolbox that fuses
developer memory (markdown), semantic code search (BM25 + optional vectors), and
AST-grep patterns, backed by a single SQLite file. No in-process LLM. No daemon.

- **Source of truth:** markdown files at `~/.comemory/memories/{id}-{slug}.md`
- **Index:** one SQLite file (`comemory.db`) with FTS5, `sqlite-vec`, and a
  typed edge graph (`edges` table)
- **Vectors are BYO.** Embeddings come from `--vector`/`--vector-stdin`; a
  sample Ollama wrapper is at `scripts/comemory-embed.sh`
- **Languages for AST extraction:** Rust, TypeScript, JavaScript, Python, Go

## Key build commands

```bash
just check                      # umbrella quality gate (fmt, type-check, lint, all checks)
just test                       # cargo nextest run --all-features
just qa                         # check + cargo-deny + dup-check + machete
just e2e                        # real-binary end-to-end harness
just fmt                        # apply rustfmt in-place
cargo install --path .          # build + install binary locally
```

## Five binding rules (non-negotiable)

These are enforced by `just check`. Every change must satisfy all five.

1. **No duplication.** Shared logic is extracted into a helper.
2. **Very modular modules.** Each file in `src/<module>/` does one narrow thing.
3. **≤300 code lines per `src/` file** (blank lines and comments excluded).
4. **Zero errors, zero warnings.** No `#[allow(...)]`, no `// clippy::allow`,
   no `.unwrap()` / `.expect()` / `todo!()` / `unimplemented!()` / `panic!` /
   `println!` / `eprintln!` in `src/`. No `unsafe` without a `// SAFETY:`
   comment within 3 lines above.
5. **Tests only in `tests/`, mirroring `src/` 1:1 and flat.** `src/<path>.rs`
   maps to `tests/<dunder-path>.rs` (e.g. `src/store/fts.rs` ↔
   `tests/store__fts.rs`). No `#[cfg(test)] mod tests` in `src/`. Shared
   helpers live in `tests/common/`.

## Code style

- 4-space indent, 100-column line length (`rustfmt.toml`)
- `cargo clippy --all-targets --all-features -- -D warnings`
- Doc comments (`///`) on every public item
- `Result<T>` from `crate::prelude::*`; errors through `src/errors.rs`
- Use `tracing` for diagnostics, never `println!`/`eprintln!`

## Module map

| Directory | Purpose |
|-----------|---------|
| `src/cli/` | clap subcommand entry points + top-level dispatcher |
| `src/memory/` | Markdown I/O, `Frontmatter`, slug, id, atomic save/load/delete |
| `src/store/` | SQLite layer: connection, schema, migrations, FTS5, vectors, custom identifier tokenizer |
| `src/graph/` | Edges table, cross-links, co-change mining (git), imports, PageRank |
| `src/retrieval/` | Search pipeline: router → rerank → diversify → bundle; ACT-R scoring, RRF fusion |
| `src/ast/` | Symbol extraction via ast-grep (5 languages) + pattern matching |
| `src/eval/` | Golden sets, recall@k/MRR metrics, reformulation mining, grid-search tuning |
| `src/tui/` | ratatui interactive terminal explorer |
| `src/stats/` | Usage/feedback/repo-marker tables |
| `src/config/` | Layered config (defaults → file → env → CLI) |
| `src/output/` | TTY and JSON emitters |
| `src/prune/` | Stale/low-value memory detection and garbage collection |
| `src/index/` | Code indexing pipeline |
| `src/serve/` | Loopback web server + embedded React SPA |
| `src/errors.rs` | Central `Error` enum (thiserror) |
| `src/prelude.rs` | Crate-internal prelude (`Error`, `Result`, common imports) |
| `src/simhash.rs` | 64-bit SimHash + Hamming distance |
| `src/git_utils.rs` | Repo/author auto-detection, git-hook helpers |

## Data model at a glance

Memories are markdown files with YAML frontmatter (`id`, `kind`, `repo`, `tags`,
`quality`, `references`, `relations`). The `edges` table is a flat
`(src_kind, src_id, edge_kind, dst_kind, dst_id, weight)` store — recursive
CTEs handle graph walks. Edge kinds include: `supersedes`, `conflicts_with`,
`derived_from`, `references_file`, `references_symbol`, `co_changed`, `imports`.

## Config layering

Defaults → `~/.comemory/config.toml` → environment variables → CLI flags.
See `CLAUDE.md` for the full env-var table (data dir, retrieval top-k, decay,
MMR lambda, BM25 weights, prune floors, etc.).

## Release process

Releases are driven by **cargo-dist** (v0.31) and triggered by pushing a
semver tag (e.g. `v0.10.1`). The `.github/workflows/release.yml` workflow:

1. Runs `scripts/check-all.sh` (the full quality gate) first.
2. Builds platform binaries via `dist build` for `aarch64-apple-darwin`,
   `aarch64-unknown-linux-gnu`, `x86_64-unknown-linux-gnu`.
3. Publishes tarballs + shell installer to the **GitHub Release**.
4. Pushes a Homebrew formula to `Falconiere/homebrew-tap` (for non-prereleases).

```bash
just release-dry-run v0.10.1   # preview the dist plan for a tag (PRs get this auto)
# Tagging triggers real release:
git tag v0.10.1 && git push origin v0.10.1
```

- Only version tags trigger publication. PRs get a dry-run plan only.
- The `[workspace.metadata.dist]` section in `Cargo.toml` configures all targets,
  installers, and the Homebrew tap.
- Every release must have a corresponding `CHANGELOG.md` entry under the
  `## <version> — YYYY-MM-DD (title)` heading convention.
- CLI reference docs (`docs/cli-reference.md`) are regenerated from `--help`
  output via `scripts/regen-cli-docs.sh`.

## Key patterns used in this codebase

- **unsafe + SAFETY:** `unsafe` is only used in `src/store/tokenizer/ffi.rs`
  (SQLite FTS5 tokenizer registration) and `src/store/connection.rs`
  (sqlite-vec extension loading). Every `unsafe` block has a `// SAFETY:`
  comment within 3 lines above explaining the invariant.
- **Error handling:** `src/errors.rs` defines a single `Error` enum (thiserror)
  plus a `Result<T>` alias. All fallible functions return `Result<T>` — never
  panic, never `.unwrap()`/`.expect()` in `src/`.
- **pub(crate) for testing:** Items that need test access are marked
  `pub(crate)` rather than guarded by `#[cfg(test)]`. No `#[cfg(test)] mod
  tests` blocks exist in `src/`.
- **tracing over println:** Use `tracing::info!`/`tracing::warn!`/
  `tracing::error!` for diagnostics. TTY output goes through `src/output/`
  (owo-colors). JSON output uses `serde_json`.
- **exit codes:** Follow `sysexits.h` conventions — `EX_OK` (0),
  `EX_DATAERR` (65), `EX_CONFIG` (78), etc.
- **Migrations:** Schema changes go in `src/store/sql/` as versioned `.sql`
  files (e.g. `0008_auto_reinforcement.sql`). Migrations are idempotent —
  they check whether each change already exists before applying.
- **Edges are flat:** No graph DB — just a single `edges` table with
  `(src_kind, src_id, edge_kind, dst_kind, dst_id, weight)`. All traversal
  uses recursive CTEs.
- **Frontmatter is the contract:** The YAML frontmatter schema (v1) in
  `src/memory/frontmatter.rs` is the public data model. Markdown files are
  the source of truth; `comemory.db` is always rebuildable from them.

## Where to start reading

- `src/main.rs` / `src/lib.rs` — binary entry + library surface
- `src/cli/mod.rs` — subcommand dispatcher
- `src/cli/save.rs` — the save flow (markdown → SQLite transaction)
- `src/retrieval/pipeline.rs` — search orchestration
- `src/store/mod.rs` — SQLite connection management
- `docs/architecture.md` — full architecture explanation
