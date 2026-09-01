//! Paginated enumeration of live memory rows from the `comemory.db` mirror.
//!
//! Pushes `comemory list`'s filters (`repo`/`kind`) and window
//! (`LIMIT`/`OFFSET`) into SQL so cost scales with the page, not the corpus
//! (the legacy path `read_dir`'d and parsed every markdown file per call).
//! Markdown stays the source of truth: this reads the mirror `cli::save` keeps
//! in sync and `comemory rebuild` reconstructs from `memories/*.md`.

use std::collections::HashMap;

use rusqlite::Connection;

use crate::prelude::*;
use crate::store::qmarks;

/// Sort order for [`list_memories`]'s window over the filtered set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortBy {
    /// `created_at DESC, id ASC` — newest created first. Today's default,
    /// unchanged by the addition of the other two orders.
    Created,
    /// `quality DESC`, ties broken by `created_at DESC, id ASC`.
    Quality,
    /// `last_accessed DESC, id ASC`. SQLite orders `NULL` as smaller than any
    /// other value, so a plain `DESC` sort already trails every
    /// never-accessed (`last_accessed IS NULL`) row without a separate
    /// `NULLS LAST` clause.
    Accessed,
}

impl SortBy {
    /// The `ORDER BY` clause body (without the `ORDER BY` keywords) for this
    /// sort.
    fn order_by(self) -> &'static str {
        match self {
            Self::Created => "created_at DESC, id ASC",
            Self::Quality => "quality DESC, created_at DESC, id ASC",
            Self::Accessed => "last_accessed DESC, id ASC",
        }
    }
}

/// One listed memory, carrying exactly the fields `comemory list` renders.
///
/// `slug` is the on-disk file stem (`{id}-{slug}`, derived from `md_path`) so
/// the value matches the legacy markdown-scan output byte-for-byte; `repo`
/// coalesces the nullable `memories.repo` column to an empty string for the
/// same reason. `body` is carried verbatim rather than a derived `title` so
/// the title stays a one-rule concept: the caller (`api::list::Row::from`)
/// derives it through `output::search::title_of`, the same helper
/// `comemory search`/`comemory show` already use, instead of the store layer
/// depending on `output` to compute it here (Binding Rule 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListRow {
    /// 8-hex memory id (`memories.id`).
    pub id: String,
    /// Canonical lowercase kind string (`memories.kind`).
    pub kind: String,
    /// Owning repo, or empty string when the memory has none.
    pub repo: String,
    /// On-disk file stem `{id}-{slug}` derived from `memories.md_path`.
    pub slug: String,
    /// Full memory body, verbatim (`memories.body`).
    pub body: String,
    /// Tag list from `memory_tags`, in row order.
    pub tags: Vec<String>,
    /// Frontmatter quality (1..=5).
    pub quality: u8,
    /// RFC 3339 creation timestamp (`memories.created_at`).
    pub created: String,
    /// Total number of times this memory has been returned by a tracked
    /// `search` / `context` run.
    pub access_count: u64,
}

/// Page of live memories plus the total count matching the same filters.
///
/// `total` is the count of rows that satisfy the `repo`/`kind` filters before
/// the `LIMIT`/`OFFSET` window is applied, so the CLI can populate
/// `Page.total` and compute an exact `has_more`.
#[derive(Debug, Clone)]
pub struct ListPage {
    /// The windowed rows, in the requested [`SortBy`] order.
    pub rows: Vec<ListRow>,
    /// Count of all rows matching the filters (pre-window).
    pub total: usize,
}

/// The optional narrowing filters for [`list_memories`], every one `AND`ed
/// into the `WHERE` clause and bound as a parameter (never interpolated).
#[derive(Debug, Clone, Copy, Default)]
pub struct ListFilter<'a> {
    /// Exact `memories.repo` match.
    pub repo: Option<&'a str>,
    /// Exact `memories.kind` match (already lowercased by the caller).
    pub kind: Option<&'a str>,
    /// Rows carrying this exact tag in `memory_tags`.
    pub tag: Option<&'a str>,
    /// Rows whose `quality` is at least this.
    pub min_quality: Option<u8>,
    /// Case-insensitive body substring (`LIKE`, with `%`/`_`/`\` escaped so
    /// the user's text is matched literally).
    pub q: Option<&'a str>,
}

/// Escape `LIKE`'s wildcard characters in a user-supplied substring so it
/// matches literally; the query pairs it with `ESCAPE '\'`.
fn like_literal(q: &str) -> String {
    let mut out = String::with_capacity(q.len() + 2);
    out.push('%');
    for c in q.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

/// List live (`deleted_at IS NULL`) memories, applying the optional
/// [`ListFilter`] narrowing and a `LIMIT`/`OFFSET` window.
///
/// `sort` selects the `ORDER BY` ([`SortBy::order_by`]); [`SortBy::Created`]
/// replicates the legacy markdown-scan sort — the fixed-width ISO-8601
/// `created_at` sorts lexicographically and the `id` tiebreak keeps the
/// window stable across pages. `limit == 0` is the shared "all" sentinel
/// ([`crate::output::page::Page::from_slice`]) — the `LIMIT` clause is
/// dropped. [`ListPage::total`] counts the filtered set before the window so
/// `has_more` is exact.
pub fn list_memories(
    conn: &Connection,
    filter: &ListFilter<'_>,
    limit: usize,
    offset: usize,
    sort: SortBy,
) -> Result<ListPage> {
    let mut filters = String::new();
    // Filter params come first; the windowed query appends the bound
    // `LIMIT`/`OFFSET` after them. Boxed so the string filters and the
    // integer window can share one `ToSql` list.
    let mut binds: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(r) = filter.repo {
        filters.push_str(" AND repo = ?");
        binds.push(Box::new(r.to_string()));
    }
    if let Some(k) = filter.kind {
        filters.push_str(" AND kind = ?");
        binds.push(Box::new(k.to_string()));
    }
    if let Some(t) = filter.tag {
        filters.push_str(" AND id IN (SELECT memory_id FROM memory_tags WHERE tag = ?)");
        binds.push(Box::new(t.to_string()));
    }
    if let Some(min) = filter.min_quality {
        filters.push_str(" AND quality >= ?");
        binds.push(Box::new(i64::from(min)));
    }
    if let Some(q) = filter.q.map(str::trim).filter(|q| !q.is_empty()) {
        filters.push_str(" AND body LIKE ? ESCAPE '\\'");
        binds.push(Box::new(like_literal(q)));
    }

    let total: usize = {
        // The COUNT carries only the filter params — never the window.
        let count_sql = format!("SELECT count(*) FROM memories WHERE deleted_at IS NULL{filters}");
        let mut stmt = conn.prepare(&count_sql)?;
        let n: i64 = stmt.query_row(
            rusqlite::params_from_iter(binds.iter().map(std::convert::AsRef::as_ref)),
            |r| r.get(0),
        )?;
        usize::try_from(n).unwrap_or(0)
    };

    // `limit == 0` means "all": SQLite forbids a bare `OFFSET`, so use its
    // `LIMIT -1` ("no limit") idiom while still honoring `offset`. Both are
    // bound params appended after the filter params.
    let limit_param: i64 = if limit == 0 {
        -1
    } else {
        i64::try_from(limit).unwrap_or(i64::MAX)
    };
    binds.push(Box::new(limit_param));
    binds.push(Box::new(i64::try_from(offset).unwrap_or(i64::MAX)));
    let order = sort.order_by();
    let sql = format!(
        "SELECT id, kind, repo, md_path, body, quality, created_at, access_count \
           FROM memories WHERE deleted_at IS NULL{filters} \
          ORDER BY {order} LIMIT ? OFFSET ?"
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt
        .query_map(
            rusqlite::params_from_iter(binds.iter().map(std::convert::AsRef::as_ref)),
            row_from_query,
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    attach_tags(conn, &mut rows)?;
    Ok(ListPage { rows, total })
}

/// Build one [`ListRow`] from a `SELECT id, kind, repo, md_path, body,
/// quality, created_at, access_count` row. `tags` starts empty; the caller
/// fills it in via [`attach_tags`] once the page's ids are known.
fn row_from_query(r: &rusqlite::Row<'_>) -> rusqlite::Result<ListRow> {
    let id: String = r.get(0)?;
    let kind: String = r.get(1)?;
    let repo: Option<String> = r.get(2)?;
    let md_path: String = r.get(3)?;
    let body: String = r.get(4)?;
    let quality: u8 = r.get(5)?;
    let created: String = r.get(6)?;
    let access_count = r.get::<_, i64>(7)?.max(0) as u64;
    Ok(ListRow {
        slug: file_stem(&md_path),
        repo: repo.unwrap_or_default(),
        id,
        kind,
        body,
        tags: Vec::new(),
        quality,
        created,
        access_count,
    })
}

/// Batch-fetch `memory_tags` rows for this page's ids and attach them to the
/// matching [`ListRow`], preserving `memory_tags` insertion order per id.
/// Scoped to the already-windowed page (not the full filtered set) so cost
/// stays proportional to the page, matching this module's `LIMIT`/`OFFSET`
/// push-down.
fn attach_tags(conn: &Connection, rows: &mut [ListRow]) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    let sql = format!(
        "SELECT memory_id, tag FROM memory_tags WHERE memory_id IN ({})",
        qmarks(ids.len())
    );
    let params: Vec<&dyn rusqlite::ToSql> =
        ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();
    let mut stmt = conn.prepare(&sql)?;
    let mut by_id: HashMap<String, Vec<String>> = HashMap::new();
    let tag_rows = stmt.query_map(params.as_slice(), |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    for row in tag_rows {
        let (id, tag) = row?;
        by_id.entry(id).or_default().push(tag);
    }
    for row in rows.iter_mut() {
        if let Some(tags) = by_id.remove(&row.id) {
            row.tags = tags;
        }
    }
    Ok(())
}

/// Extract the file stem (`{id}-{slug}`) from a stored `md_path`, matching the
/// legacy `Path::file_stem` behavior of the markdown-scan listing: strip the
/// directory prefix and a single trailing extension. Pure string work so the
/// listing never touches the filesystem.
fn file_stem(md_path: &str) -> String {
    let name = md_path.rsplit(['/', '\\']).next().unwrap_or(md_path);
    match name.rsplit_once('.') {
        Some((stem, _ext)) if !stem.is_empty() => stem.to_string(),
        _ => name.to_string(),
    }
}

#[cfg(test)]
#[path = "tests/memory_list.rs"]
mod tests;
