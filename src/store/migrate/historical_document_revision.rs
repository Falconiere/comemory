//! Repair the document table written by the short-lived branch that also
//! recorded `0026_repository_approval`. Its migration 25 used a staging
//! lifecycle; the released migration 25 stores only accepted revisions.

use std::collections::BTreeSet;

use rusqlite::Connection;

use crate::prelude::*;

const POST_MARKER: &str = "0028_historical_document_revision_repair";

/// Complete the v28 recovery after its SQL marker is recorded. The repair
/// and its own marker share one transaction, so an interruption can retry.
pub(super) fn repair(conn: &mut Connection) -> Result<()> {
    if super::marker_done(conn, POST_MARKER) {
        return Ok(());
    }
    let columns: BTreeSet<String> = conn
        .prepare("SELECT name FROM pragma_table_info('remote_document')")?
        .query_map([], |row| row.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    let current = columns.contains("accepted_at") && !columns.contains("state");
    let historical = !columns.contains("accepted_at")
        && ["state", "created_at", "activated_at"]
            .iter()
            .all(|column| columns.contains(*column));
    if !current && !historical {
        return Err(Error::Migration(
            "0028_historical_document_revision: remote_document has an unrecognized shape".into(),
        ));
    }
    let tx = conn.transaction()?;
    if historical {
        tx.execute_batch(REPAIR_SQL)?;
    }
    super::insert_marker(&tx, POST_MARKER)?;
    tx.commit()?;
    Ok(())
}

/// Build the released table shape and retain only revisions the historical
/// build marked active. Its staged and superseded rows must not
/// become searchable merely because the new schema has no state column.
const REPAIR_SQL: &str = "
CREATE TABLE remote_document_repaired (
    repo TEXT NOT NULL,
    shared_id TEXT NOT NULL,
    path TEXT NOT NULL,
    title TEXT NOT NULL,
    format TEXT NOT NULL CHECK (format IN ('txt', 'markdown', 'html', 'delimited')),
    revision_hash TEXT NOT NULL,
    chunk_count INTEGER NOT NULL,
    accepted_at TEXT NOT NULL,
    PRIMARY KEY (repo, shared_id)
);
INSERT INTO remote_document_repaired
    (repo, shared_id, path, title, format, revision_hash, chunk_count, accepted_at)
SELECT repo, shared_id, path, title, format, revision_hash, chunk_count,
       COALESCE(activated_at, created_at)
  FROM remote_document WHERE state = 'active';
DROP TABLE remote_document;
ALTER TABLE remote_document_repaired RENAME TO remote_document;
CREATE INDEX idx_remote_document_path ON remote_document (repo, path);
DELETE FROM remote_document_chunk
 WHERE NOT EXISTS (SELECT 1 FROM remote_document d
                    WHERE d.repo = remote_document_chunk.repo
                      AND d.shared_id = remote_document_chunk.shared_id);
DELETE FROM remote_document_link
 WHERE NOT EXISTS (SELECT 1 FROM remote_document d
                    WHERE d.repo = remote_document_link.repo
                      AND d.shared_id = remote_document_link.shared_id);
DELETE FROM remote_document_fts
 WHERE NOT EXISTS (SELECT 1 FROM remote_document d
                    WHERE d.repo = remote_document_fts.repo
                      AND d.shared_id = remote_document_fts.shared_id);
";
