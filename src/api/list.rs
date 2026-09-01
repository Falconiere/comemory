//! `api::list::{Request, run}` — the shared middle of `comemory list` /
//! `GET /api/v1/memories`: page live memories from the SQLite mirror with
//! optional `repo`/`kind` filters. Moved out of `cli::list::run` so the CLI
//! and the HTTP route call one implementation (Binding Rule 1).

use serde::{Deserialize, Serialize};

use crate::api::Ctx;
use crate::output::page::Page;
use crate::output::search::title_of;
use crate::prelude::*;
use crate::store::memory_list::{self, ListFilter, ListRow, SortBy};

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
    /// Filter to memories carrying this exact tag.
    #[serde(default)]
    pub tag: Option<String>,
    /// Filter to memories whose quality is at least this (1..=5).
    #[serde(default)]
    pub min_quality: Option<u8>,
    /// Filter to memories whose body contains this text (case-insensitive
    /// substring, matched literally). `query` is accepted as an alias so the
    /// CLI's `--query` flag and the console's `?q=` name the same field.
    #[serde(default, alias = "query")]
    pub q: Option<String>,
    /// Maximum number of results to return. `0` means "all" (no limit).
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Number of leading results to skip before the window starts.
    #[serde(default)]
    pub offset: usize,
    /// Sort order: `created` (default, newest first) | `quality`
    /// (descending) | `accessed` (most-recently-accessed first).
    #[serde(default)]
    pub sort: Sort,
}

/// `PaginationArgs`' CLI default (`--limit`, unset = 50), reused here so an
/// HTTP request omitting `limit` pages identically to the CLI.
fn default_limit() -> usize {
    50
}

/// Sort order for `comemory list` / `GET /api/v1/memories` rows. Mirrors
/// `cli::list::Sort`'s three values as a serde enum (rather than a shared
/// type) so the CLI keeps its `clap::ValueEnum` derive and the HTTP surface
/// keeps a plain `Deserialize` one; [`SortBy`] is the store-layer type both
/// map onto via [`From`]. The console-api spec's names are accepted as
/// aliases: `recent` → `created`, `activation` → `accessed` (access recency
/// is the ordering ACT-R activation is dominated by, and the one the mirror
/// can sort in SQL).
#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Sort {
    /// Newest created first — today's default ordering, unchanged.
    #[default]
    #[serde(alias = "recent")]
    Created,
    /// Descending quality.
    Quality,
    /// Most-recently-accessed first; never-accessed rows sort last.
    #[serde(alias = "activation")]
    Accessed,
}

impl From<Sort> for SortBy {
    fn from(s: Sort) -> Self {
        match s {
            Sort::Created => Self::Created,
            Sort::Quality => Self::Quality,
            Sort::Accessed => Self::Accessed,
        }
    }
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
    /// First non-empty trimmed line of the body.
    pub title: String,
    /// Tag list from `memory_tags`.
    pub tags: Vec<String>,
    /// Frontmatter quality (1..=5).
    pub quality: u8,
    /// RFC 3339 creation timestamp.
    pub created: String,
    /// Total number of times this memory has been returned by a tracked
    /// `search` / `context` run.
    pub access_count: u64,
}

impl From<ListRow> for Row {
    fn from(r: ListRow) -> Self {
        Self {
            title: title_of(&r.body),
            id: r.id,
            kind: r.kind,
            repo: r.repo,
            slug: r.slug,
            tags: r.tags,
            quality: r.quality,
            created: r.created,
            access_count: r.access_count,
        }
    }
}

/// List filtered memories from the SQLite mirror as a paginated `Page<Row>`.
/// `kind` is matched case-insensitively against the canonical lowercase
/// `memories.kind` values, mirroring the CLI's `--kind` behavior.
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Page<Row>> {
    if let Some(q) = req.min_quality
        && !(1..=5).contains(&q)
    {
        return Err(Error::BadRequest(format!(
            "min_quality must be in 1..=5, got {q}"
        )));
    }
    let kind = req.kind.as_deref().map(str::to_ascii_lowercase);
    let filter = ListFilter {
        repo: req.repo.as_deref(),
        kind: kind.as_deref(),
        tag: req.tag.as_deref(),
        min_quality: req.min_quality,
        q: req.q.as_deref(),
    };
    let conn = ctx.conn()?;
    let listed = memory_list::list_memories(conn, &filter, req.limit, req.offset, req.sort.into())?;
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
