//! `api::index_runs` — `GET /api/v1/index/runs`: the paged `index_runs`
//! history (console-api spec §6).
//!
//! A pure reader over `store::index_runs::list`: one row per `index-code`
//! run (CLI and HTTP alike), newest first, optionally narrowed to one repo
//! label.
//!
//! **Must-not-create-the-db invariant** (the same rule `api::repos` and
//! `api::stats` keep): being asked for the run history must never create
//! and migrate a database as a side effect. On a data dir with no
//! `comemory.db`, [`run`] never calls [`Ctx::conn`] and reports an empty
//! page.

use serde::Deserialize;

use crate::api::Ctx;
use crate::output::page::Page;
use crate::prelude::*;
use crate::store::index_runs::{self, IndexRunRow};

/// Rows per page when the caller sends no `limit` — a console history list,
/// not a search result, so the window is wider than the retrieval default.
const DEFAULT_LIMIT: usize = 50;

/// [`DEFAULT_LIMIT`] as a serde `default` provider.
fn default_limit() -> usize {
    DEFAULT_LIMIT
}

/// `GET /api/v1/index/runs` request.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Narrow the history to one repo label; every repo when absent.
    #[serde(default)]
    pub repo: Option<String>,
    /// Page size, defaulting to [`DEFAULT_LIMIT`]. `0` is the shared "all"
    /// sentinel every paged command honors.
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Rows skipped before this window.
    #[serde(default)]
    pub offset: usize,
}

impl Default for Request {
    /// The unfiltered first page — `limit` is [`DEFAULT_LIMIT`], NOT the
    /// `0` an `#[derive(Default)]` would produce (which is the "all"
    /// sentinel and would silently page differently from the HTTP default).
    fn default() -> Self {
        Self {
            repo: None,
            limit: DEFAULT_LIMIT,
            offset: 0,
        }
    }
}

/// One `(limit, offset)` window of the run history, newest first. See the
/// module doc for why a missing database is an empty page rather than a
/// freshly-created one.
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Page<IndexRunRow>> {
    if !ctx.paths.db_path().exists() {
        return Ok(Page::new(Vec::new(), req.limit, req.offset, Some(0), false));
    }
    let conn = ctx.conn()?;
    let (rows, total) = index_runs::list(conn, req.repo.as_deref(), req.limit, req.offset)?;
    let has_more = req.offset.saturating_add(rows.len()) < total;
    Ok(Page::new(
        rows,
        req.limit,
        req.offset,
        Some(total),
        has_more,
    ))
}

#[cfg(test)]
#[path = "tests/index_runs.rs"]
mod tests;
