//! `api::list::{Request, run}` — the shared middle of `comemory list` /
//! `GET /api/v1/memories`: page live memories from the SQLite mirror with
//! optional `repo`/`kind` filters. Moved out of `cli::list::run` so the CLI
//! and the HTTP route call one implementation (Binding Rule 1).

use serde::{Deserialize, Serialize};

use crate::api::Ctx;
use crate::output::page::Page;
use crate::prelude::*;
use crate::store::memory_list::{self, ListRow};

/// `comemory list` / `GET /api/v1/memories` request. Every field is
/// optional — an empty request lists every live memory, newest first.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Filter to memories whose `repo` matches exactly.
    #[serde(default)]
    pub repo: Option<String>,
    /// Filter by kind (case-insensitive): decision|bug|convention|discovery|pattern|note.
    #[serde(default)]
    pub kind: Option<String>,
    /// Maximum number of results to return. `0` means "all" (no limit).
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Number of leading results to skip before the window starts.
    #[serde(default)]
    pub offset: usize,
}

/// `PaginationArgs`' CLI default (`--limit`, unset = 50), reused here so an
/// HTTP request omitting `limit` pages identically to the CLI.
fn default_limit() -> usize {
    50
}

/// One row of `comemory list` / `GET /api/v1/memories` output.
#[derive(Serialize)]
pub struct Row {
    /// 8-hex memory id.
    pub id: String,
    /// Canonical lowercase kind string.
    pub kind: String,
    /// Owning repo, or empty string when the memory has none.
    pub repo: String,
    /// On-disk file stem `{id}-{slug}`.
    pub slug: String,
}

impl From<ListRow> for Row {
    fn from(r: ListRow) -> Self {
        Self {
            id: r.id,
            kind: r.kind,
            repo: r.repo,
            slug: r.slug,
        }
    }
}

/// List filtered memories from the SQLite mirror as a paginated `Page<Row>`.
/// `kind` is matched case-insensitively against the canonical lowercase
/// `memories.kind` values, mirroring the CLI's `--kind` behavior.
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Page<Row>> {
    let kind = req.kind.as_deref().map(str::to_ascii_lowercase);
    let conn = ctx.conn()?;
    let listed = memory_list::list_memories(
        conn,
        req.repo.as_deref(),
        kind.as_deref(),
        req.limit,
        req.offset,
    )?;
    let rows: Vec<Row> = listed.rows.into_iter().map(Row::from).collect();
    let has_more = req.offset + rows.len() < listed.total;
    Ok(Page::new(
        rows,
        req.limit,
        req.offset,
        Some(listed.total),
        has_more,
    ))
}
