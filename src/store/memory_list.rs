//! Paginated enumeration of live memory rows from the `comemory.db` mirror.
//!
//! Pushes `comemory list`'s filters (`repo`/`kind`) and window
//! (`LIMIT`/`OFFSET`) into SQL so cost scales with the page, not the corpus
//! (the legacy path `read_dir`'d and parsed every markdown file per call).
//! Markdown stays the source of truth: this reads the mirror `cli::save` keeps
//! in sync and `comemory rebuild` reconstructs from `memories/*.md`.

use rusqlite::Connection;

use crate::prelude::*;

/// One listed memory, carrying exactly the fields `comemory list` renders.
///
/// `slug` is the on-disk file stem (`{id}-{slug}`, derived from `md_path`) so
/// the value matches the legacy markdown-scan output byte-for-byte; `repo`
/// coalesces the nullable `memories.repo` column to an empty string for the
/// same reason.
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
}

/// Page of live memories plus the total count matching the same filters.
///
/// `total` is the count of rows that satisfy the `repo`/`kind` filters before
/// the `LIMIT`/`OFFSET` window is applied, so the CLI can populate
/// `Page.total` and compute an exact `has_more`.
#[derive(Debug, Clone)]
pub struct ListPage {
    /// The windowed rows, in `created_at DESC, id ASC` order.
    pub rows: Vec<ListRow>,
    /// Count of all rows matching the filters (pre-window).
    pub total: usize,
}

/// List live (`deleted_at IS NULL`) memories, applying optional exact
/// `repo`/`kind` filters and a `LIMIT`/`OFFSET` window.
///
/// Order is `created_at DESC, id ASC`, replicating the legacy markdown-scan
/// sort; the fixed-width ISO-8601 `created_at` sorts lexicographically and the
/// `id` tiebreak keeps the window stable across pages. `limit == 0` is the
/// shared "all" sentinel ([`crate::output::page::Page::from_slice`]) — the
/// `LIMIT` clause is dropped. [`ListPage::total`] counts the filtered set
/// before the window so `has_more` is exact.
pub fn list_memories(
    conn: &Connection,
    repo: Option<&str>,
    kind: Option<&str>,
    limit: usize,
    offset: usize,
) -> Result<ListPage> {
    let mut filters = String::new();
    // Filter params (`repo`/`kind`) come first; the windowed query appends the
    // bound `LIMIT`/`OFFSET` after them. Boxed so the string filters and the
    // integer window can share one `ToSql` list.
    let mut binds: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(r) = repo {
        filters.push_str(" AND repo = ?");
        binds.push(Box::new(r.to_string()));
    }
    if let Some(k) = kind {
        filters.push_str(" AND kind = ?");
        binds.push(Box::new(k.to_string()));
    }

    let total: usize = {
        // The COUNT carries only the filter params — never the window.
        let count_sql = format!("SELECT count(*) FROM memories WHERE deleted_at IS NULL{filters}");
        let mut stmt = conn.prepare(&count_sql)?;
        let n: i64 = stmt.query_row(
            rusqlite::params_from_iter(binds.iter().map(|b| b.as_ref())),
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
    let sql = format!(
        "SELECT id, kind, repo, md_path FROM memories \
          WHERE deleted_at IS NULL{filters} \
          ORDER BY created_at DESC, id ASC LIMIT ? OFFSET ?"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(
            rusqlite::params_from_iter(binds.iter().map(|b| b.as_ref())),
            |r| {
                let id: String = r.get(0)?;
                let kind: String = r.get(1)?;
                let repo: Option<String> = r.get(2)?;
                let md_path: String = r.get(3)?;
                Ok(ListRow {
                    slug: file_stem(&md_path),
                    repo: repo.unwrap_or_default(),
                    id,
                    kind,
                })
            },
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(ListPage { rows, total })
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
