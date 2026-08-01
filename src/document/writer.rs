//! Per-file index writer: the fast fingerprint check (size+mtime → SHA-256
//! confirm → extract → one-transaction row replacement) that keeps
//! `source_files`/`documents`/`document_chunks`/`document_fts` current for
//! one discovered candidate, plus the tombstone reconciliation that
//! retires rows an authoritative scan no longer sees. Mirrors the
//! code-index seam (`src/cli/index_code.rs`) but commits its own dedicated
//! transaction PER FILE rather than one transaction for a whole walk — see
//! the design spec's "Index lifecycle and freshness" section.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use rusqlite::Connection;
use time::OffsetDateTime;

use super::fingerprint::{self, FileStat};
use super::{DocumentFormat, ExtractedDocument, extract};
use crate::graph::doc_link;
use crate::graph::edges;
use crate::prelude::*;
use crate::source::classify::Classification;
use crate::source::discover::Candidate;
use crate::store::document_fts;
use crate::store::documents::{self, ChunkRow, DocumentUpsert};
use crate::store::memory_row::iso_format;
use crate::store::sources::{self, SourceFileRow, SourceFileUpsert};

/// Outcome of [`update_file`] for one candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateOutcome {
    /// Fingerprint (or confirmed hash) unchanged; nothing rewritten.
    Unchanged,
    /// Extraction ran and the document's rows were replaced.
    Indexed {
        /// Stable 32-hex-char id of the `documents` row.
        document_id: String,
    },
    /// File exceeds `max_file_bytes`; recorded, not extracted.
    TooLarge,
    /// [`Classification::Unsupported`]; recorded for diagnostics only.
    Unsupported,
    /// [`Classification::Ignored`]; nothing written.
    Ignored,
    /// A stat/read/extract failure. A read or extract failure records a
    /// typed `source_files` diagnostic and leaves the previous
    /// revision's `documents`/`document_chunks`/`document_fts` rows
    /// intact; a bare stat failure (the file vanished between
    /// discovery and now) writes nothing at all.
    Error(String),
}

/// Update one discovered candidate's derived rows under `source_id`.
/// Dispatches on [`Candidate::classification`]; see [`UpdateOutcome`]
/// for what each branch does.
pub fn update_file(
    conn: &mut Connection,
    source_id: &str,
    repo: Option<&str>,
    candidate: &Candidate,
    source_root: &Path,
    max_file_bytes: u64,
) -> Result<UpdateOutcome> {
    match candidate.classification {
        Classification::Ignored => Ok(UpdateOutcome::Ignored),
        Classification::Unsupported => record_unsupported(conn, source_id, candidate),
        Classification::Document(format) => update_document(
            conn,
            source_id,
            repo,
            candidate,
            format,
            source_root,
            max_file_bytes,
        ),
    }
}

/// Stat-then-record an unsupported candidate: no extraction, no
/// `documents` row, just a `source_files` diagnostic row. A stat
/// failure (the file vanished between discovery and now) writes
/// nothing and reports [`UpdateOutcome::Error`].
fn record_unsupported(
    conn: &Connection,
    source_id: &str,
    candidate: &Candidate,
) -> Result<UpdateOutcome> {
    let relative_path = candidate.relative_path.to_string_lossy().into_owned();
    let meta = match fs::metadata(&candidate.absolute_path) {
        Ok(m) => m,
        Err(e) => return Ok(UpdateOutcome::Error(format!("stat: {e}"))),
    };
    let stat = FileStat {
        source_id,
        relative_path: &relative_path,
        file_id: &fingerprint::file_id(source_id, &relative_path),
        size: meta.len() as i64,
        mtime: fingerprint::mtime_nanos(&meta)?,
    };
    fingerprint::upsert_stat(conn, &stat, "unsupported", "unsupported", None, None)?;
    Ok(UpdateOutcome::Unsupported)
}

/// The fingerprint → SHA-256 confirm → extract → one-transaction write
/// flow for one [`Classification::Document`] candidate (design spec:
/// "Index lifecycle and freshness", steps 1-4).
fn update_document(
    conn: &mut Connection,
    source_id: &str,
    repo: Option<&str>,
    candidate: &Candidate,
    format: DocumentFormat,
    source_root: &Path,
    max_file_bytes: u64,
) -> Result<UpdateOutcome> {
    let relative_path = candidate.relative_path.to_string_lossy().into_owned();
    let owned_file_id = fingerprint::file_id(source_id, &relative_path);
    let meta = match fs::metadata(&candidate.absolute_path) {
        Ok(m) => m,
        Err(e) => return Ok(UpdateOutcome::Error(format!("stat: {e}"))),
    };
    let stat = FileStat {
        source_id,
        relative_path: &relative_path,
        file_id: &owned_file_id,
        size: meta.len() as i64,
        mtime: fingerprint::mtime_nanos(&meta)?,
    };
    let existing = sources::get_file(conn, stat.file_id)?;
    if let Some(outcome) =
        fingerprint::fingerprint_shortcut(conn, &stat, existing.as_ref(), max_file_bytes)?
    {
        return Ok(outcome);
    }
    let bytes = match read_within_boundary(&candidate.absolute_path, source_root) {
        Ok(b) => b,
        Err(e) => {
            let msg = format!("read: {e}");
            fingerprint::upsert_stat(conn, &stat, "document", "error", None, Some(&msg))?;
            return Ok(UpdateOutcome::Error(msg));
        }
    };
    let hash = fingerprint::content_hash(&bytes);
    if existing.is_some_and(|r| {
        r.sha256.as_deref() == Some(hash.as_str())
            && fingerprint::was_successfully_processed(&r.status)
    }) {
        sources::touch_file(conn, stat.file_id, stat.size, stat.mtime, &iso_now()?)?;
        return Ok(UpdateOutcome::Unchanged);
    }
    extract_and_write(conn, repo, &stat, format, &bytes, &hash)
}

/// Re-resolve `absolute_path` and read it only if the resolved target
/// still lives inside `source_root` — closes the TOCTOU window between
/// `source::discover`'s walk-time symlink validation and this later
/// read. Reads through the resolved path, not `absolute_path`, so the
/// read itself doesn't re-traverse the symlink. `source_root` is also
/// re-canonicalized here rather than trusted from the caller, so a
/// future non-canonical root can't weaken or falsely trip the boundary.
/// Any canonicalize/read failure, or the boundary violation itself,
/// surfaces as the same `io::Error` the caller treats as a read failure.
fn read_within_boundary(absolute_path: &Path, source_root: &Path) -> std::io::Result<Vec<u8>> {
    let resolved = fs::canonicalize(absolute_path)?;
    let root = fs::canonicalize(source_root)?;
    if !resolved.starts_with(&root) {
        return Err(std::io::Error::other(format!(
            "resolved target escaped source root: {}",
            resolved.display()
        )));
    }
    fs::read(&resolved)
}

/// Extract `bytes` and, on success, replace the document's rows in one
/// transaction; on failure, record a `source_files` diagnostic and
/// leave the previous revision's `documents`/`document_chunks`/
/// `document_fts` rows untouched (spec: "previous revision rows left
/// intact").
fn extract_and_write(
    conn: &mut Connection,
    repo: Option<&str>,
    stat: &FileStat<'_>,
    format: DocumentFormat,
    bytes: &[u8],
    hash: &str,
) -> Result<UpdateOutcome> {
    let file_stem = file_stem_of(stat.relative_path);
    let extracted = match extract::extract(format, bytes, &file_stem) {
        Ok(d) => d,
        Err(e) => {
            fingerprint::upsert_stat(
                conn,
                stat,
                "document",
                "error",
                Some(hash),
                Some(&e.to_string()),
            )?;
            return Ok(UpdateOutcome::Error(e.to_string()));
        }
    };
    let document_id = fingerprint::document_id_of(stat.file_id);
    write_indexed(conn, repo, stat, &document_id, hash, &extracted)?;
    // Design spec: "After a file's transaction commits, the link deriver
    // writes edges" — deliberately AFTER `write_indexed`'s own transaction,
    // not inside it.
    doc_link::derive_after_document(
        conn,
        stat.source_id,
        stat.file_id,
        &document_id,
        repo,
        stat.relative_path,
        &extracted.links,
    )?;
    Ok(UpdateOutcome::Indexed { document_id })
}

/// The writer's ONE transaction: `source_files` (status `indexed`),
/// `documents`, and a full `document_chunks`/`document_fts` replace.
fn write_indexed(
    conn: &mut Connection,
    repo: Option<&str>,
    stat: &FileStat<'_>,
    document_id: &str,
    hash: &str,
    extracted: &ExtractedDocument,
) -> Result<()> {
    let now = iso_now()?;
    let tx = conn.transaction()?;
    sources::upsert_file(
        &tx,
        SourceFileUpsert {
            id: stat.file_id,
            source_id: stat.source_id,
            relative_path: stat.relative_path,
            classification: "document",
            size: stat.size,
            mtime: stat.mtime,
            sha256: Some(hash),
            status: "indexed",
            error: None,
            created_at: &now,
            updated_at: &now,
        },
    )?;
    documents::upsert_document(
        &tx,
        DocumentUpsert {
            id: document_id,
            source_file_id: stat.file_id,
            title: &extracted.title,
            repo,
            revision_hash: hash,
            created_at: &now,
            updated_at: &now,
        },
    )?;
    write_chunks(&tx, document_id, stat.relative_path, extracted)?;
    tx.commit()?;
    Ok(())
}

/// Replace `document_id`'s `document_chunks` + `document_fts` rows from
/// `extracted.chunks`. `relative_path` becomes every chunk's
/// `path_tokens`; `extracted.title` becomes every chunk's FTS title.
fn write_chunks(
    conn: &Connection,
    document_id: &str,
    relative_path: &str,
    extracted: &ExtractedDocument,
) -> Result<()> {
    let headings: Vec<String> = extracted
        .chunks
        .iter()
        .map(|c| c.heading_path.join(" > "))
        .collect();
    let rows: Vec<ChunkRow<'_>> = extracted
        .chunks
        .iter()
        .zip(&headings)
        .map(|(c, h)| ChunkRow {
            ordinal: c.ordinal as i64,
            heading_path: h.as_str(),
            char_range: (c.char_range.0 as i64, c.char_range.1 as i64),
            line_range: (c.line_range.0 as i64, c.line_range.1 as i64),
            simhash: c.simhash as i64,
            text: c.text.as_str(),
        })
        .collect();
    documents::replace_chunks(conn, document_id, &rows)?;
    document_fts::delete_document(conn, document_id)?;
    for (c, h) in extracted.chunks.iter().zip(&headings) {
        document_fts::insert(
            conn,
            document_id,
            c.ordinal as i64,
            &extracted.title,
            h,
            &c.text,
            relative_path,
        )?;
    }
    Ok(())
}

/// Given the FULL set of relative paths an authoritative discovery walk
/// of `source_id` just saw, tombstone every `source_files` row for that
/// source NOT in `seen`: its `documents`/`document_chunks`/
/// `document_fts` rows are removed and its own row flips to `deleted`
/// (spec step 5). A row already `deleted` is left alone — idempotent.
/// Returns the number of rows newly tombstoned.
pub fn reconcile_deletions(
    conn: &mut Connection,
    source_id: &str,
    seen: &HashSet<String>,
) -> Result<usize> {
    let mut removed = 0usize;
    for row in sources::list_files_by_source(conn, source_id)? {
        if row.status == "deleted" || seen.contains(&row.relative_path) {
            continue;
        }
        tombstone(conn, &row)?;
        removed += 1;
    }
    Ok(removed)
}

/// Remove `row`'s derived document rows (plus the `member_of_source` /
/// `references_document` edges it owns — `edges` has no FK, so a soft
/// delete must purge them explicitly) and flip it to `deleted`, all in
/// one transaction.
fn tombstone(conn: &mut Connection, row: &SourceFileRow) -> Result<()> {
    let document_id = fingerprint::document_id_of(&row.id);
    let now = iso_now()?;
    let tx = conn.transaction()?;
    documents::delete_document(&tx, &document_id)?;
    document_fts::delete_document(&tx, &document_id)?;
    edges::delete_touching(&tx, "file", &row.id)?;
    edges::delete_touching(&tx, "document", &document_id)?;
    sources::mark_deleted(&tx, &row.id, &now)?;
    tx.commit()?;
    Ok(())
}

/// The file stem of `relative_path` (its own file name for a
/// single-file source) — the extractor's title fallback when the
/// format carries no heading.
fn file_stem_of(relative_path: &str) -> String {
    Path::new(relative_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(relative_path)
        .to_string()
}

/// Current wall-clock time as the RFC3339/ISO8601 string every
/// `created_at`/`updated_at` column stores (`store::memory_row`'s
/// house format). `pub(super)` so [`super::fingerprint::upsert_stat`]
/// shares this one house-format implementation rather than its own.
pub(super) fn iso_now() -> Result<String> {
    iso_format(OffsetDateTime::now_utc())
}
