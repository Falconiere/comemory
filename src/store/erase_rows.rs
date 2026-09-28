//! The rows a permanent erase (#256, B-5) removes, and the storage steps that
//! make the removal stick: `secure_delete` on the erasing connection, FTS5
//! `optimize` so deleted postings leave the index segments, and a truncating
//! WAL checkpoint so no frame still holds the pre-erase pages.
//!
//! What an erase keeps — feed rows, revisions, receipts, digests — is not
//! touched here; the payload bytes are blanked by
//! [`super::replica_redaction`], their replay scratch by
//! [`super::replica_redaction_copies`], and their complete staged sets here.
//! Hand SQL: `PRAGMA`s, FTS5 commands, one join and the pulled-cache deletes
//! across repositories, none of which the declared builders express.

use std::time::{Duration, Instant};

use rusqlite::{Connection, OptionalExtension};
use toolu_orm::core::query_column::CommonOps;

use super::candidate_observations::MemoryRedaction;
use super::schema_documents::{SourceFiles, source_files};
use super::schema_memory::{Memories, memories};
use super::schema_replica::{ReplicaStagedPart, replica_staged_part};
use super::{
    activity_share, candidate_observations, document_fts, documents, edges, memory_intent,
    memory_purge, needs_embedding, orm, replica_staging,
};
use crate::prelude::*;
use crate::utilities::canonical_json;

/// What the mirror holds for one memory before it is erased.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryState {
    /// `deleted_at IS NULL` — the memory is live rather than trashed.
    pub live: bool,
    /// Body digest the legacy tombstone row carries.
    pub content_hash: String,
    /// Repo label, empty when unscoped.
    pub repo: String,
}

/// The `memories` row for `id`, live or trashed, if there is one.
///
/// # Errors
/// Propagates SQLite failures.
pub fn memory_state(conn: &Connection, id: &str) -> Result<Option<MemoryState>> {
    orm::query_optional(
        conn,
        Memories::select()
            .columns_typed(&[
                &memories::deleted_at,
                &memories::content_hash,
                &memories::repo,
            ])
            .filter(memories::id.eq(id))
            .to_sql(),
        |r| {
            Ok(MemoryState {
                live: r.get::<_, Option<String>>(0)?.is_none(),
                content_hash: r.get(1)?,
                repo: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
            })
        },
    )
}

/// Delete every row keyed by memory `id`, whatever its state: the `memories`
/// row, everything [`memory_purge::purge_rows`] removes, the embedding
/// backlog and write-intent rows, the display locators of its captured
/// candidates, and the title a run's summary recorded for it. In the
/// caller's transaction. Returns the verdict digests erased.
///
/// # Errors
/// Propagates SQLite failures.
pub fn erase_memory(tx: &Connection, id: &str, at: &str) -> Result<Vec<String>> {
    orm::execute(tx, Memories::delete().filter(memories::id.eq(id)).to_sql())?;
    let digests = memory_purge::purge_rows(tx, id, at)?;
    needs_embedding::clear(tx, id)?;
    memory_intent::clear(tx, id)?;
    candidate_observations::redact_memory_candidates(tx, id, MemoryRedaction::Locator)?;
    activity_share::scrub_titles(tx, id)?;
    Ok(digests)
}

/// One local document an erase reaches through its shared name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalDocument {
    /// `documents.id`.
    pub document_id: String,
    /// The `source_files` row the document was extracted from.
    pub source_file_id: String,
}

/// Every local document shared (or withheld) under `shared_id`.
///
/// # Errors
/// Propagates SQLite failures.
pub fn local_documents(conn: &Connection, shared_id: &str) -> Result<Vec<LocalDocument>> {
    let mut statement = conn.prepare(
        "SELECT s.document_id, d.source_file_id FROM document_share s \
           JOIN documents d ON d.id = s.document_id \
          WHERE s.shared_id = ?1 ORDER BY s.document_id",
    )?;
    let rows = statement
        .query_map([shared_id], |r| {
            Ok(LocalDocument {
                document_id: r.get(0)?,
                source_file_id: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Delete one local document's derived rows — `documents` (cascading its
/// chunks and its `document_share` row), `document_fts`, the edges its file
/// and document nodes carry — and its `source_files` row, so a later index
/// of the untouched source file reads it as new. In the caller's
/// transaction; the file itself is never touched.
///
/// # Errors
/// Propagates SQLite failures.
pub fn erase_local_document(tx: &Connection, doc: &LocalDocument) -> Result<()> {
    documents::delete_document(tx, &doc.document_id)?;
    document_fts::delete_document(tx, &doc.document_id)?;
    edges::delete_touching(tx, "file", &doc.source_file_id)?;
    edges::delete_touching(tx, "document", &doc.document_id)?;
    orm::execute(
        tx,
        SourceFiles::delete()
            .filter(source_files::id.eq(doc.source_file_id.as_str()))
            .to_sql(),
    )?;
    Ok(())
}

/// The pulled-cache tables, each keyed by `(repo, shared_id)`; the virtual
/// FTS table first, since nothing cascades to it.
const PULLED_TABLES: [&str; 4] = [
    "remote_document_fts",
    "remote_document_chunk",
    "remote_document_link",
    "remote_document",
];

/// Delete every pulled copy of `shared_id`, under whichever repositories
/// hold one, in the caller's transaction. Returns whether any row went.
///
/// # Errors
/// Propagates SQLite failures.
pub fn erase_pulled_document(tx: &Connection, shared_id: &str) -> Result<bool> {
    let mut removed = 0usize;
    for table in PULLED_TABLES {
        removed += tx.execute(
            &format!("DELETE FROM {table} WHERE shared_id = ?1"),
            [shared_id],
        )?;
    }
    Ok(removed > 0)
}

/// Delete every COMPLETE staged-part set whose assembled bytes hash to one of
/// `digests` — the canonical digest activation computes — so an upload of an
/// erased payload cannot activate it later. An incomplete set cannot be
/// attributed and is left to [`super::replica_sweep`]. Returns parts deleted.
///
/// # Errors
/// Propagates SQLite failures.
pub fn erase_staged_of(conn: &Connection, digests: &[String]) -> Result<u64> {
    let staging_ids: Vec<String> = orm::query_all(
        conn,
        ReplicaStagedPart::select()
            .columns_typed(&[&replica_staged_part::staging_id])
            .distinct()
            .to_sql(),
        |r| r.get(0),
    )?;
    let doomed = staging_ids
        .into_iter()
        .map(|id| Ok((assembled_digest(conn, &id)?, id)))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .filter_map(|(digest, id)| digest.filter(|d| digests.contains(d)).map(|_| id));
    doomed
        .map(|id| Ok(replica_staging::discard(conn, &id)? as u64))
        .sum()
}

/// The canonical digest of one staged set's assembled bytes — `None` while a
/// declared part is missing, or when the bytes are not JSON.
fn assembled_digest(conn: &Connection, staging_id: &str) -> Result<Option<String>> {
    let Some(bytes) = replica_staging::assemble(conn, staging_id)? else {
        return Ok(None);
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&bytes) else {
        return Ok(None);
    };
    Ok(Some(canonical_json::bytes_and_digest(&value)?.1))
}

/// An FTS5 index an erase may have left deleted postings in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FtsIndex {
    /// `memory_fts`.
    Memories,
    /// `memory_substring` (trigrams over `memories.body`).
    MemorySubstring,
    /// `edge_fts`, rewritten by the derived-state refresh.
    Edges,
    /// `document_fts`.
    Documents,
    /// `remote_document_fts`.
    PulledDocuments,
}

impl FtsIndex {
    /// The virtual table's name — a closed set, never caller text.
    fn table(self) -> &'static str {
        match self {
            Self::Memories => "memory_fts",
            Self::MemorySubstring => "memory_substring",
            Self::Edges => "edge_fts",
            Self::Documents => "document_fts",
            Self::PulledDocuments => "remote_document_fts",
        }
    }
}

/// Merge `index` into one segment, dropping every deleted posting — without
/// it the erased text's terms stay in the older segments until an automerge
/// happens to reach them.
///
/// # Errors
/// Propagates SQLite failures.
pub fn optimize(conn: &Connection, index: FtsIndex) -> Result<()> {
    let table = index.table();
    conn.execute(
        &format!("INSERT INTO {table}({table}) VALUES('optimize')"),
        [],
    )?;
    Ok(())
}

/// Run `work` with `PRAGMA secure_delete = ON`, so every cell, overflow page
/// and freed page it deletes is overwritten with zeros, then put back the
/// connection's prior setting — this may be a server's shared connection.
/// The first error wins; the setting is restored either way.
///
/// # Errors
/// Propagates `work`'s error and SQLite failures setting the pragma.
pub fn with_secure_delete<T>(
    conn: &mut Connection,
    work: impl FnOnce(&mut Connection) -> Result<T>,
) -> Result<T> {
    let prior: i64 = conn.pragma_query_value(None, "secure_delete", |r| r.get(0))?;
    conn.pragma_update(None, "secure_delete", 1_i64)?;
    let result = work(conn);
    let restored = conn.pragma_update(None, "secure_delete", prior);
    let value = result?;
    restored?;
    Ok(value)
}

/// How long [`truncate_wal`] sleeps between attempts.
const CHECKPOINT_POLL: Duration = Duration::from_millis(50);

/// `wal_checkpoint(TRUNCATE)`, retried until `wait` elapses while a reader
/// holds an open transaction. `true` once the WAL is empty (or the database
/// is not in WAL mode at all); `false` when a reader outlasted `wait` — the
/// erase still committed, and the frames go at the next checkpoint.
///
/// # Errors
/// Propagates SQLite failures other than a lock held by another connection.
pub fn truncate_wal(conn: &Connection, wait: Duration) -> Result<bool> {
    let deadline = Instant::now() + wait;
    loop {
        let attempt = conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
                r.get::<_, i64>(0)
            })
            .optional()
            .map_err(Error::from);
        match attempt {
            Ok(Some(0) | None) => return Ok(true),
            Ok(Some(_)) => {}
            Err(e) if super::busy::is_locked(&e) => {}
            Err(e) => return Err(e),
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        std::thread::sleep(CHECKPOINT_POLL);
    }
}

#[cfg(test)]
#[path = "tests/erase_rows.rs"]
mod tests;
