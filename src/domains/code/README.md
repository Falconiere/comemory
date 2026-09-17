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
| `ast.rs` + `ast/` | `extract` | Symbol extraction and AST pattern search via ast-grep (rust/ts/js/py/go): the per-language extractor, cAST chunking of oversized symbols, the user-facing pattern API, and the process-global compiled-pattern cache |
| `git_utils.rs` | `repo_label` | Blob OID / HEAD / branch / remote lookups, Git-hook installation helpers, and the one rule that turns a checkout into a repo label (the main worktree's basename, via the Git common directory) |

`src/lib.rs` re-exports `ast` and `git_utils` at the crate root so
`comemory::ast` and `comemory::git_utils` keep resolving for external consumers;
in-crate code names `crate::domains::code::<name>` directly.

Colocated unit tests live in `tests/` beside their module and are reached through
each module's `#[path]` bridge.
