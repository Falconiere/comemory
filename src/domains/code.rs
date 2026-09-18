//! `domains::code` — everything about mirroring a repository's symbols and
//! keeping that mirror fresh.
//!
//! Extraction ([`ast`]) turns source files into symbol rows; [`index_code`] and
//! [`ingest_code`] write them through `store`; [`repos`] and [`repo_admin`] own
//! the repository inventory; [`git_utils`] answers the Git questions the rest of
//! the capability asks (repo label, HEAD, blob OIDs); [`hooks`] and
//! [`self::install_hooks`] own the reindex hooks; [`reindex_policy`] decides when a
//! lazy reindex is due. `store` keeps every SQL string, and the CLI owns the
//! detached process launch.

/// Symbol extraction and AST pattern search via ast-grep.
pub mod ast;
/// Repo/author detection, blob lookup, and Git-hook installation helpers.
pub mod git_utils;
/// `comemory hooks`: read and toggle the git reindex hooks.
pub mod hooks;
/// `comemory index-code` (DB-write path): mirror a repo's symbols.
pub mod index_code;
/// `GET /index/runs`: the `index_runs` history.
pub mod index_runs;
/// `comemory ingest-code`: mirror pre-embedded NDJSON symbol rows.
pub mod ingest_code;
/// `comemory install-hooks`: install git hooks for background reindexing.
pub mod install_hooks;
/// `comemory ast`: run an ast-grep pattern against one file, paged.
pub mod pattern_search;
/// When a lazy auto-reindex is due: staleness, debounce, and repo context.
pub mod reindex_policy;
/// `POST /repos`, `PATCH /repos/{name}`, archive, `DELETE /repos/{name}`.
pub mod repo_admin;
/// `comemory repos`: the indexed code-repository inventory.
pub mod repos;
