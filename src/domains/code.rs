//! `domains::code` — everything about mirroring a repository's symbols and
//! keeping that mirror fresh.
//!
//! Extraction ([`ast`][ast]) turns source files into symbol rows;
//! [`index_code`][index_code] and [`ingest_code`][ingest_code] write them
//! through `store`; [`repos`][repos] and [`repo_admin`][repo_admin] own the
//! repository inventory; [`git_utils`][git_utils] answers the Git questions the
//! rest of the capability asks (repo label, HEAD, blob OIDs);
//! [`hooks`][hooks] and [`install_hooks`][install_hooks] own the reindex hooks;
//! [`reindex_policy`][reindex_policy] decides when a lazy reindex is due.
//! `store` keeps every SQL string, and the CLI owns the detached process
//! launch.
//!
//! Every link above is written in reference style against a fully-qualified
//! path. A capability file carries `///` on each `pub mod` line, which merges
//! this module doc into the parent's resolution scope: a bare `[`ast`]` then
//! resolves in module `domains`, not `domains::code`, and rustdoc reports
//! "no item named `ast` in module `domains`". Nine links here did exactly that
//! until #178.
//!
//! [ast]: crate::domains::code::ast
//! [index_code]: crate::domains::code::index_code
//! [ingest_code]: crate::domains::code::ingest_code
//! [repos]: crate::domains::code::repos
//! [repo_admin]: crate::domains::code::repo_admin
//! [git_utils]: crate::domains::code::git_utils
//! [hooks]: crate::domains::code::hooks
//! [install_hooks]: crate::domains::code::install_hooks
//! [reindex_policy]: crate::domains::code::reindex_policy

/// Symbol extraction and AST pattern search via ast-grep.
pub mod ast;
/// Repo/author detection, blob lookup, and Git-hook installation helpers.
pub mod generation;
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
pub mod replica_payload;
/// `POST /repos`, `PATCH /repos/{name}`, archive, `DELETE /repos/{name}`.
pub mod repo_admin;
/// `comemory repos`: the indexed code-repository inventory.
pub mod repos;
