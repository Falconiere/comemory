//! `api::trash` — `GET /api/v1/trash`: soft-deleted memories with their
//! days until gc (console-api spec §9).
//!
//! A soft-deleted memory is a `memories` row with `deleted_at` set and its
//! markdown moved into `memories/.trash/`. `comemory gc` reaps a trashed
//! file once its **mtime** is older than `prune.trash_retention_days`, so
//! that is the clock this listing counts down — the `deleted_at` stamp is
//! only a fallback for a row whose file is already gone (hard-deleted by
//! hand, or reaped between the two reads).
//!
//! Read-only, and it never creates `comemory.db`: a data dir with no
//! database has nothing in the trash and answers an empty page.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::api::Ctx;
use crate::output::page::Page;
use crate::output::search::title_of;
use crate::prelude::*;
use crate::retrieval::score;

/// `GET /api/v1/trash` request.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Maximum number of rows to return. `0` means "all".
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Number of leading rows to skip before the window starts.
    #[serde(default)]
    pub offset: usize,
}

/// The paged-command default page size, matching `api::list`.
fn default_limit() -> usize {
    50
}

/// One soft-deleted memory awaiting gc.
#[derive(Serialize, Debug)]
pub struct TrashRow {
    /// 8-hex memory id.
    pub id: String,
    /// First non-empty trimmed line of the body.
    pub title: String,
    /// Canonical lowercase kind string.
    pub kind: String,
    /// Owning repo, or `None` when the memory has none.
    pub repo: Option<String>,
    /// RFC 3339 timestamp the memory was soft-deleted at.
    pub deleted_at: String,
    /// Path of the file inside `memories/.trash/`, or `None` when the row is
    /// soft-deleted but its markdown is no longer on disk.
    pub path: Option<String>,
    /// Whole days left before `comemory gc` may reap the trashed file,
    /// clamped at `0` (already eligible).
    pub days_until_gc: i64,
}

/// The `memories` columns this listing reads, before the on-disk join.
struct RawRow {
    id: String,
    body: String,
    kind: String,
    repo: Option<String>,
    deleted_at: String,
}

/// List soft-deleted memories, newest deletion first. Never creates the
/// database (see the module doc): a missing `comemory.db` answers an empty
/// page rather than migrating one into existence for a read.
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Page<TrashRow>> {
    if !ctx.paths.db_path().exists() {
        return Ok(Page::from_slice(Vec::new(), req.limit, req.offset));
    }
    let retention = i64::from(ctx.cfg.prune.trash_retention_days);
    let files = trash_files(&ctx.paths.trash_dir());
    let now = OffsetDateTime::now_utc();
    let conn = ctx.conn()?;
    let rows: Vec<TrashRow> = deleted_rows(conn)?
        .into_iter()
        .map(|raw| {
            let path = files.get(&raw.id);
            TrashRow {
                title: title_of(&raw.body),
                days_until_gc: days_until_gc(path, &raw.deleted_at, retention, now),
                path: path.map(|p| p.to_string_lossy().into_owned()),
                id: raw.id,
                kind: raw.kind,
                repo: raw.repo,
                deleted_at: raw.deleted_at,
            }
        })
        .collect();
    Ok(Page::from_slice(rows, req.limit, req.offset))
}

/// Every soft-deleted `memories` row, ordered newest deletion first with the
/// id as a stable tie-breaker (so paging is deterministic when a batch of
/// memories was deleted in the same run).
fn deleted_rows(conn: &Connection) -> Result<Vec<RawRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, body, kind, repo, deleted_at FROM memories \
          WHERE deleted_at IS NOT NULL ORDER BY deleted_at DESC, id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(RawRow {
            id: r.get(0)?,
            body: r.get(1)?,
            kind: r.get(2)?,
            repo: r.get(3)?,
            deleted_at: r.get(4)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Index `memories/.trash/` by memory id (`{id}-{slug}.md`). A missing or
/// unreadable trash directory yields an empty map — every row then falls
/// back to its `deleted_at` stamp.
fn trash_files(trash_dir: &Path) -> HashMap<String, PathBuf> {
    let mut out = HashMap::new();
    let Ok(entries) = std::fs::read_dir(trash_dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().into_string().unwrap_or_default();
        let is_md = Path::new(&name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("md"));
        if let Some((id, _)) = name.split_once('-')
            && is_md
        {
            out.insert(id.to_string(), entry.path());
        }
    }
    out
}

/// Days left before gc may reap this entry: the retention window minus the
/// whole days elapsed on the file's mtime (the clock `api::gc::sweep_trash`
/// actually reads), falling back to `deleted_at` when the file is gone.
/// Clamped at `0` — an overdue entry reports `0`, never a negative count.
fn days_until_gc(
    path: Option<&PathBuf>,
    deleted_at: &str,
    retention: i64,
    now: OffsetDateTime,
) -> i64 {
    let elapsed = path
        .and_then(|p| mtime_days(p))
        .unwrap_or_else(|| score::days_since(deleted_at, now));
    (retention - elapsed as i64).max(0)
}

/// Whole and fractional days since `path`'s mtime; `None` when the file
/// cannot be stat'd or its mtime is in the future (an unusable clock).
fn mtime_days(path: &Path) -> Option<f64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(modified.elapsed().ok()?.as_secs() as f64 / 86_400.0)
}

#[cfg(test)]
#[path = "tests/trash.rs"]
mod tests;
