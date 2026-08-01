//! `source_roots` row CRUD — the SQLite mirror of `sources.toml`
//! (`src/source/registry.rs`). Carries only ephemeral state (status,
//! timestamps); the TOML file stays authoritative for identity, and
//! `src/source/mirror.rs` reconciles this table from it.

use rusqlite::{Connection, OptionalExtension, params};

use crate::prelude::*;

/// Upsert SQL for one `source_roots` row. `ON CONFLICT(id)` leaves
/// `status` and `created_at` untouched, so a reconcile pass never resets
/// a row a later reachability check marked `unreachable`, nor its
/// original registration timestamp.
const UPSERT_SQL: &str = "INSERT INTO source_roots(\
     id, canonical_path, kind, repo, status, created_at, updated_at) \
 VALUES(?1,?2,?3,?4,'active',?5,?6) \
 ON CONFLICT(id) DO UPDATE SET \
     canonical_path = excluded.canonical_path, kind = excluded.kind, \
     repo = excluded.repo, updated_at = excluded.updated_at";

/// Caller-supplied fields for [`upsert`].
pub struct SourceRootUpsert<'a> {
    /// 32-hex-char [`crate::source::SourceId`] string.
    pub id: &'a str,
    /// Absolute, symlink-resolved source path.
    pub canonical_path: &'a str,
    /// `"file"` or `"dir"`.
    pub kind: &'a str,
    /// Optional repository label.
    pub repo: Option<&'a str>,
    /// RFC3339/ISO8601 timestamp used only on first insert.
    pub created_at: &'a str,
    /// RFC3339/ISO8601 timestamp, always refreshed.
    pub updated_at: &'a str,
}

/// One `source_roots` row as read back from SQLite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRootRow {
    /// 32-hex-char source id.
    pub id: String,
    /// Absolute, symlink-resolved source path.
    pub canonical_path: String,
    /// `"file"` or `"dir"`.
    pub kind: String,
    /// Optional repository label.
    pub repo: Option<String>,
    /// `"active"` or `"unreachable"`.
    pub status: String,
    /// Row creation timestamp.
    pub created_at: String,
    /// Row last-update timestamp.
    pub updated_at: String,
}

/// Insert a fresh `source_roots` row (`status = 'active'`), or refresh an
/// existing row's mutable fields, leaving its `status` and `created_at`
/// untouched.
pub fn upsert(conn: &Connection, row: SourceRootUpsert<'_>) -> Result<()> {
    conn.execute(
        UPSERT_SQL,
        params![
            row.id,
            row.canonical_path,
            row.kind,
            row.repo,
            row.created_at,
            row.updated_at
        ],
    )?;
    Ok(())
}

/// Delete a `source_roots` row by id. Cascades to `source_files` /
/// `documents` / `document_chunks` via `ON DELETE CASCADE`.
pub fn delete(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM source_roots WHERE id = ?1", params![id])?;
    Ok(())
}

/// List every `source_roots` row, ordered by `canonical_path` for a
/// deterministic listing.
pub fn list(conn: &Connection) -> Result<Vec<SourceRootRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, canonical_path, kind, repo, status, created_at, updated_at \
           FROM source_roots ORDER BY canonical_path",
    )?;
    let rows = stmt
        .query_map([], row_from_sql)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Fetch one `source_roots` row by id, or `None` if it does not exist.
pub fn get(conn: &Connection, id: &str) -> Result<Option<SourceRootRow>> {
    conn.query_row(
        "SELECT id, canonical_path, kind, repo, status, created_at, updated_at \
           FROM source_roots WHERE id = ?1",
        params![id],
        row_from_sql,
    )
    .optional()
    .map_err(Error::from)
}

/// Total `source_roots` row count.
pub fn count(conn: &Connection) -> Result<usize> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM source_roots", [], |r| r.get(0))?;
    Ok(n as usize)
}

fn row_from_sql(r: &rusqlite::Row<'_>) -> rusqlite::Result<SourceRootRow> {
    Ok(SourceRootRow {
        id: r.get(0)?,
        canonical_path: r.get(1)?,
        kind: r.get(2)?,
        repo: r.get(3)?,
        status: r.get(4)?,
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
    })
}

/// Caller-supplied fields for [`upsert_file`]. `id` is caller-computed —
/// deterministic per `(source_id, relative_path)`, see
/// [`crate::document::writer::file_id`] — so the `ON CONFLICT(id)`
/// upsert below is exactly "update this file's existing row if any".
pub struct SourceFileUpsert<'a> {
    /// 32-hex-char file id.
    pub id: &'a str,
    /// Owning [`crate::source::SourceId`] string.
    pub source_id: &'a str,
    /// Path relative to the source root's canonical path.
    pub relative_path: &'a str,
    /// `"document"`, `"ignored"`, or `"unsupported"`.
    pub classification: &'a str,
    /// File size in bytes, from the writer's stat.
    pub size: i64,
    /// Modification time, nanoseconds since the Unix epoch.
    pub mtime: i64,
    /// Full 64-hex-char SHA-256 of the file's content; `None` until a
    /// changed fingerprint forces the writer to compute it.
    pub sha256: Option<&'a str>,
    /// `"pending"`, `"indexed"`, `"stale"`, `"too_large"`, `"error"`,
    /// `"unsupported"`, or `"deleted"`.
    pub status: &'a str,
    /// Typed diagnostic; `None` when `status` carries none.
    pub error: Option<&'a str>,
    /// RFC3339/ISO8601 timestamp used only on first insert.
    pub created_at: &'a str,
    /// RFC3339/ISO8601 timestamp, always refreshed.
    pub updated_at: &'a str,
}

/// One `source_files` row as read back from SQLite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFileRow {
    /// 32-hex-char file id.
    pub id: String,
    /// Owning source id.
    pub source_id: String,
    /// Path relative to the source root.
    pub relative_path: String,
    /// `"document"`, `"ignored"`, or `"unsupported"`.
    pub classification: String,
    /// File size in bytes at last fingerprint.
    pub size: i64,
    /// Modification time, nanoseconds since the Unix epoch.
    pub mtime: i64,
    /// Full 64-hex-char SHA-256 of the file's content, if confirmed.
    pub sha256: Option<String>,
    /// Current lifecycle status.
    pub status: String,
    /// Typed diagnostic, if `status` carries one.
    pub error: Option<String>,
    /// Row creation timestamp.
    pub created_at: String,
    /// Row last-update timestamp.
    pub updated_at: String,
}

const FILE_UPSERT_SQL: &str = "INSERT INTO source_files(\
     id, source_id, relative_path, classification, size, mtime, sha256, \
     status, error, created_at, updated_at) \
 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) \
 ON CONFLICT(id) DO UPDATE SET \
     classification = excluded.classification, size = excluded.size, \
     mtime = excluded.mtime, sha256 = excluded.sha256, \
     status = excluded.status, error = excluded.error, \
     updated_at = excluded.updated_at";

const FILE_SELECT_COLUMNS: &str = "id, source_id, relative_path, classification, \
     size, mtime, sha256, status, error, created_at, updated_at";

/// Insert a fresh `source_files` row, or overwrite an existing one's
/// mutable fields (everything but `id`/`source_id`/`relative_path`/
/// `created_at`, which stay fixed once a row exists).
pub fn upsert_file(conn: &Connection, row: SourceFileUpsert<'_>) -> Result<()> {
    conn.execute(
        FILE_UPSERT_SQL,
        params![
            row.id,
            row.source_id,
            row.relative_path,
            row.classification,
            row.size,
            row.mtime,
            row.sha256,
            row.status,
            row.error,
            row.created_at,
            row.updated_at,
        ],
    )?;
    Ok(())
}

/// Narrow "fingerprint touch": update only `size`/`mtime`/`updated_at`
/// on an existing row, leaving `status`/`sha256`/`error` untouched. Used
/// when a SHA-256 confirm finds unchanged content behind a changed
/// mtime — the writer's "touch fingerprint, skip" path.
pub fn touch_file(
    conn: &Connection,
    id: &str,
    size: i64,
    mtime: i64,
    updated_at: &str,
) -> Result<()> {
    conn.execute(
        "UPDATE source_files SET size = ?2, mtime = ?3, updated_at = ?4 WHERE id = ?1",
        params![id, size, mtime, updated_at],
    )?;
    Ok(())
}

/// Flip a row to the `deleted` tombstone status, leaving its last-known
/// `size`/`mtime`/`sha256` in place for diagnostics (spec: "a `deleted`
/// tombstone for diagnostics").
pub fn mark_deleted(conn: &Connection, id: &str, updated_at: &str) -> Result<()> {
    conn.execute(
        "UPDATE source_files SET status = 'deleted', updated_at = ?2 WHERE id = ?1",
        params![id, updated_at],
    )?;
    Ok(())
}

/// Fetch one `source_files` row by id, or `None` if it does not exist.
pub fn get_file(conn: &Connection, id: &str) -> Result<Option<SourceFileRow>> {
    let sql = format!("SELECT {FILE_SELECT_COLUMNS} FROM source_files WHERE id = ?1");
    conn.query_row(&sql, params![id], file_row_from_sql)
        .optional()
        .map_err(Error::from)
}

/// List every `source_files` row for one source, ordered by
/// `relative_path` for a deterministic scan — the authoritative-scan
/// reconciliation in [`crate::document::writer::reconcile_deletions`]
/// diffs this against a fresh discovery walk.
pub fn list_files_by_source(conn: &Connection, source_id: &str) -> Result<Vec<SourceFileRow>> {
    let sql = format!(
        "SELECT {FILE_SELECT_COLUMNS} FROM source_files WHERE source_id = ?1 ORDER BY relative_path"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![source_id], file_row_from_sql)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Per-status `source_files` counts for one source, surfaced by
/// `comemory sources` (the report's `indexed`/`error`/`stale` columns).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SourceFileCounts {
    /// Rows with `status = 'indexed'`.
    pub indexed: usize,
    /// Rows with `status = 'error'`.
    pub error: usize,
    /// Rows with `status = 'stale'` (the lazy-freshness unreachable-root
    /// marker; no writer sets this status yet).
    pub stale: usize,
}

/// Count `source_files` rows for `source_id` by status, narrowed to the
/// three statuses `comemory sources` reports. Every other status
/// (`pending`/`too_large`/`unsupported`/`deleted`) is intentionally
/// uncounted here — they are visible via `comemory index`'s own report.
pub fn file_status_counts(conn: &Connection, source_id: &str) -> Result<SourceFileCounts> {
    let mut counts = SourceFileCounts::default();
    let mut stmt = conn.prepare(
        "SELECT status, COUNT(*) FROM source_files WHERE source_id = ?1 GROUP BY status",
    )?;
    let rows = stmt.query_map(params![source_id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (status, n) = row?;
        let n = n as usize;
        match status.as_str() {
            "indexed" => counts.indexed = n,
            "error" => counts.error = n,
            "stale" => counts.stale = n,
            _ => {}
        }
    }
    Ok(counts)
}

fn file_row_from_sql(r: &rusqlite::Row<'_>) -> rusqlite::Result<SourceFileRow> {
    Ok(SourceFileRow {
        id: r.get(0)?,
        source_id: r.get(1)?,
        relative_path: r.get(2)?,
        classification: r.get(3)?,
        size: r.get(4)?,
        mtime: r.get(5)?,
        sha256: r.get(6)?,
        status: r.get(7)?,
        error: r.get(8)?,
        created_at: r.get(9)?,
        updated_at: r.get(10)?,
    })
}
