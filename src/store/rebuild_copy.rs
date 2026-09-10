//! The `ATTACH`-based preservation copy lifecycle behind
//! `api::rebuild::copy`: [`copy_preserved_tables_from_old`] owns attach,
//! every per-table copy pass, and detach as ONE unit, so no caller can pair
//! attach/detach incorrectly and leave a database attached to a live
//! connection on an early return.
//!
//! The three copy passes — code-index (`rebuild_copy_code`), learning-loop
//! (`rebuild_copy_learning`, which itself calls `rebuild_copy_history`), and
//! document-domain (`rebuild_copy_documents`) — are split into sibling
//! files so none crosses the 300-line ceiling; this file is the entry point
//! plus the two schema-probe helpers every pass shares.

use std::path::Path;

use crate::prelude::*;
use crate::store::{Connection, rebuild_copy_code, rebuild_copy_documents, rebuild_copy_learning};

/// Attach `old_db` as `old` and copy the code-index, learning, and
/// document-domain tables into `conn` (the freshly built tmp database).
/// Each source table is copied only if it exists on the attached DB.
///
/// Caller must populate `main.source_roots` first
/// (`source::mirror::reconcile`): `source_files.source_id` is a foreign
/// key, so the document-domain copy fails outright if its parent is
/// missing. `DETACH` always runs, even on a copy failure, so the
/// connection stays reusable — its own result is discarded via `let _ =`.
pub fn copy_preserved_tables_from_old(conn: &mut Connection, old_db: &Path) -> Result<()> {
    conn.execute(
        "ATTACH DATABASE ? AS old",
        rusqlite::params![old_db.to_string_lossy().as_ref()],
    )?;
    let copy_result = rebuild_copy_code::copy_code_tables_inner(conn)
        .and_then(|()| rebuild_copy_learning::copy_learning_tables_inner(conn))
        .and_then(|()| rebuild_copy_documents::copy_document_tables_inner(conn));
    // Always attempt DETACH so the connection is reusable even if the copy
    // failed. A DETACH failure is logged, never propagated: the copy's own
    // outcome is what the caller acts on, and turning a SUCCESSFUL copy into
    // an error because the cleanup stumbled would abort a rebuild that had
    // already done its work. Logging keeps it from being silent.
    if let Err(e) = conn.execute_batch("DETACH DATABASE old;") {
        tracing::warn!(error = %e, "DETACH DATABASE old failed after the preservation copy");
    }
    copy_result
}

/// Copy one table's rows wholesale from the attached `old` database, naming
/// every column explicitly — `SELECT *` is not reliable for every FTS5 shape
/// across an ATTACH. Skips silently when `old` predates the table, which is
/// how a pre-v0.2 database is carried across.
///
/// `table` and `columns` are `&'static str`: interpolated into the SQL, so
/// only a compile-time literal at a call site can reach them. There is no
/// dynamic value in this statement to bind.
pub(crate) fn copy_table(
    conn: &Connection,
    table: &'static str,
    columns: &'static str,
) -> Result<()> {
    if old_table_exists(conn, table)? {
        conn.execute_batch(&format!(
            "INSERT OR IGNORE INTO main.{table}({columns}) SELECT {columns} FROM old.{table};"
        ))?;
    }
    Ok(())
}

/// True when `name` exists as a table (regular or virtual) on the attached
/// `old` database. Lets every copy pass skip tables that predate v0.2.
pub(crate) fn old_table_exists(conn: &Connection, name: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT count(*) FROM old.sqlite_master WHERE type = 'table' AND name = ?1",
        rusqlite::params![name],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

/// True when `column` exists on `table` in the attached `old` database.
/// Lets a copy pass adapt its SELECT list to the attached DB's schema
/// version instead of assuming the current one.
pub(crate) fn old_column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT count(*) FROM pragma_table_info(?1, 'old') WHERE name = ?2",
        rusqlite::params![table, column],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

#[cfg(test)]
#[path = "tests/rebuild_copy.rs"]
mod tests;
