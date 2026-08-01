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
use std::time::UNIX_EPOCH;

use rusqlite::Connection;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

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

/// Common identity + freshly-taken fingerprint for one candidate,
/// bundled so the write-path helpers below don't each carry a long
/// argument list.
struct FileStat<'a> {
    source_id: &'a str,
    relative_path: &'a str,
    file_id: &'a str,
    size: i64,
    mtime: i64,
}

/// Update one discovered candidate's derived rows under `source_id`.
/// Dispatches on [`Candidate::classification`]; see [`UpdateOutcome`]
/// for what each branch does.
pub fn update_file(
    conn: &mut Connection,
    source_id: &str,
    repo: Option<&str>,
    candidate: &Candidate,
    max_file_bytes: u64,
) -> Result<UpdateOutcome> {
    match candidate.classification {
        Classification::Ignored => Ok(UpdateOutcome::Ignored),
        Classification::Unsupported => record_unsupported(conn, source_id, candidate),
        Classification::Document(format) => {
            update_document(conn, source_id, repo, candidate, format, max_file_bytes)
        }
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
        file_id: &file_id(source_id, &relative_path),
        size: meta.len() as i64,
        mtime: mtime_nanos(&meta)?,
    };
    upsert_stat(conn, &stat, "unsupported", "unsupported", None, None)?;
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
    max_file_bytes: u64,
) -> Result<UpdateOutcome> {
    let relative_path = candidate.relative_path.to_string_lossy().into_owned();
    let owned_file_id = file_id(source_id, &relative_path);
    let meta = match fs::metadata(&candidate.absolute_path) {
        Ok(m) => m,
        Err(e) => return Ok(UpdateOutcome::Error(format!("stat: {e}"))),
    };
    let stat = FileStat {
        source_id,
        relative_path: &relative_path,
        file_id: &owned_file_id,
        size: meta.len() as i64,
        mtime: mtime_nanos(&meta)?,
    };
    let existing = sources::get_file(conn, stat.file_id)?;
    if let Some(outcome) = fingerprint_shortcut(conn, &stat, existing.as_ref(), max_file_bytes)? {
        return Ok(outcome);
    }
    let bytes = match fs::read(&candidate.absolute_path) {
        Ok(b) => b,
        Err(e) => {
            let msg = format!("read: {e}");
            upsert_stat(conn, &stat, "document", "error", None, Some(&msg))?;
            return Ok(UpdateOutcome::Error(msg));
        }
    };
    let hash = content_hash(&bytes);
    if existing.is_some_and(|r| r.sha256.as_deref() == Some(hash.as_str())) {
        sources::touch_file(conn, stat.file_id, stat.size, stat.mtime, &iso_now()?)?;
        return Ok(UpdateOutcome::Unchanged);
    }
    extract_and_write(conn, repo, &stat, format, &bytes, &hash)
}

/// Fast pre-read checks: exact-fingerprint skip, then the size ceiling.
/// `Ok(None)` means "proceed to read + hash the content".
fn fingerprint_shortcut(
    conn: &Connection,
    stat: &FileStat<'_>,
    existing: Option<&SourceFileRow>,
    max_file_bytes: u64,
) -> Result<Option<UpdateOutcome>> {
    if existing.is_some_and(|r| r.size == stat.size && r.mtime == stat.mtime) {
        return Ok(Some(UpdateOutcome::Unchanged));
    }
    if stat.size as u64 > max_file_bytes {
        upsert_stat(conn, stat, "document", "too_large", None, None)?;
        return Ok(Some(UpdateOutcome::TooLarge));
    }
    Ok(None)
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
            upsert_stat(
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
    let document_id = document_id_of(stat.file_id);
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

/// Upsert a `source_files` row for `stat` with the given
/// `classification`/`status`/`sha256`/`error`. Shared by every
/// `source_files`-only write path (`unsupported`, `too_large`,
/// `error`).
fn upsert_stat(
    conn: &Connection,
    stat: &FileStat<'_>,
    classification: &str,
    status: &str,
    sha256: Option<&str>,
    error: Option<&str>,
) -> Result<()> {
    let now = iso_now()?;
    sources::upsert_file(
        conn,
        SourceFileUpsert {
            id: stat.file_id,
            source_id: stat.source_id,
            relative_path: stat.relative_path,
            classification,
            size: stat.size,
            mtime: stat.mtime,
            sha256,
            status,
            error,
            created_at: &now,
            updated_at: &now,
        },
    )
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
    let document_id = document_id_of(&row.id);
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

/// Nanoseconds since the Unix epoch for `meta`'s modification time —
/// the `source_files.mtime` fast-fingerprint column (pinned as
/// nanoseconds by the design plan).
fn mtime_nanos(meta: &fs::Metadata) -> Result<i64> {
    let modified = meta.modified()?;
    let nanos = modified
        .duration_since(UNIX_EPOCH)
        .map_err(|e| Error::Other(format!("mtime before unix epoch: {e}")))?
        .as_nanos();
    Ok(nanos as i64)
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
/// house format).
fn iso_now() -> Result<String> {
    iso_format(OffsetDateTime::now_utc())
}

/// Full 64-hex-char SHA-256 of `bytes` — the writer's SHA-256 confirm
/// step, and the value stored as both `source_files.sha256` and
/// `documents.revision_hash`.
fn content_hash(bytes: &[u8]) -> String {
    hex_of(&Sha256::digest(bytes))
}

/// Deterministic 32-hex-char (128-bit) id for one candidate file within
/// a source: the first 16 bytes of `SHA-256(source_id ++ NUL ++
/// relative_path)`, hex-encoded — the same "hash the identity" idiom
/// [`crate::source::SourceId::from_canonical_path`] uses. Stable across
/// re-index runs of the same file, which is what lets
/// [`sources::upsert_file`]'s `ON CONFLICT(id)` update-not-duplicate
/// the row. `pub(crate)` so a later CLI step can pre-derive it (e.g.
/// for `comemory unindex`).
pub(crate) fn file_id(source_id: &str, relative_path: &str) -> String {
    let digest = Sha256::digest(format!("{source_id}\u{0}{relative_path}").as_bytes());
    hex_of(&digest[..16])
}

/// Deterministic 32-hex-char document id, derived from its owning
/// [`file_id`] so the id survives a content edit at the same path
/// (spec: "ID survives content edits at the same path") without a row
/// lookup — `file_id` is itself invariant across a content edit. A path
/// RENAME therefore mints a new document id; full rename detection via
/// a cross-file content-hash match (the spec's "otherwise delete+create"
/// clause) is deferred past this writer.
pub(crate) fn document_id_of(file_id: &str) -> String {
    let digest = Sha256::digest(file_id.as_bytes());
    hex_of(&digest[..16])
}

/// Lowercase-hex encode `bytes`.
fn hex_of(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}
