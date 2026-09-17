# `domains/code/`

**What belongs here:** mirroring a repository's symbols into the code index and
keeping that mirror fresh — extraction, indexing, the repository inventory, the
Git probes those need, the reindex hooks, and the staleness policy behind a lazy
reindex.

**What does NOT belong here:** SQL, and delivery. Every `rusqlite` import and SQL
string stays in `store` (`store::code_row`, `store::code_graph_*`,
`store::indexed_files`, `store::repo_marker`, …); this folder calls those. The
detached `index-code` process launch stays in `cli::lazy_reindex`, argument
parsing and rendering stay in `cli`, and HTTP status/envelope mapping stays in
`serve`. Ranking code hits is retrieval's job, not this folder's.

## Contents

One line per file, named after its primary item:

| File | Primary item | Owns |
| --- | --- | --- |
| `ast.rs` + `ast/` | `extract` | Symbol extraction and AST pattern search via ast-grep (rust/ts/js/py/go): the per-language extractor, cAST chunking of oversized symbols, the pattern-matching primitive, and the process-global compiled-pattern cache |
| `git_utils.rs` | `repo_label` | Blob OID / HEAD / branch / remote lookups, Git-hook installation helpers, and the one rule that turns a checkout into a repo label (the main worktree's basename, via the Git common directory) |
| `hooks.rs` | `Request` | Shared middle of `comemory hooks` / `GET\|POST /api/v1/hooks` — per-hook read and toggle over the two state stores: the three reindex hooks on disk under the Git common directory, and the config-backed search→edit reinforcement row |
| `index_code.rs` + `index_code/` | `Request` | Shared middle of `comemory index-code`'s DB-write path / `POST /api/v1/code/index`, incl. the incremental/full `mode` switch, cancellation, the PageRank post-pass, and the `index_runs` row every run records; the per-file walk and its blob-OID skip live in `index_code/walk.rs` |
| `index_runs.rs` | `Request` | Console-only: `GET /api/v1/index/runs` — the paged `index_runs` history, newest first, and never creating the database to answer |
| `ingest_code.rs` | `Response` | Shared middle of `comemory ingest-code` / `POST /api/v1/code/ingest` — NDJSON symbol rows parsed into `code_symbols` + `code_fts` + `code_vec` |
| `install_hooks.rs` | `Request` | Shared middle of `comemory install-hooks` / `POST /api/v1/hooks/install` — preflight every target first, then refresh the comemory-owned reindex hooks, leaving foreign hooks alone unless forced |
| `pattern_search.rs` | `Request` | Shared middle of `comemory ast` / `POST /api/v1/code/ast` — one ast-grep pattern against one source file, paged `(line, text)` matches; conn-free |
| `reindex_policy.rs` | `should_reindex` | The pure half of the lazy reindex: the staleness/debounce decision, the `schema_meta` trigger marker (`lazy_reindex_head:<repo>`), and the repo label / working-tree resolution it needs — no process or terminal I/O, so the detached launch stays in `cli::lazy_reindex` |
| `repo_admin.rs` | `ConnectRequest` | Console-only: `POST /api/v1/repos`, `PATCH /repos/{name}`, `POST /repos/{name}/archive`, `DELETE /repos/{name}` — the explicit repo lifecycle the CLI gets implicitly by running `index-code` |
| `repos.rs` + `repos/` | `Request` | Shared middle of `comemory repos` / `GET /api/v1/repos` — the `repo_marker` join plus per-repo counters; the git freshness probe lives in `repos/git_state.rs` and degrades to `status: "unknown"` rather than failing the inventory |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/domains/code.rs` (`pub mod
<name>;`) and callers import concrete paths.

`src/lib.rs` re-exports `ast` and `git_utils` at the crate root so
`comemory::ast` and `comemory::git_utils` keep resolving for external consumers;
in-crate code names `crate::domains::code::<name>` directly.

Colocated unit tests live in `tests/` beside their module and are reached through
each module's `#[path]` bridge.
