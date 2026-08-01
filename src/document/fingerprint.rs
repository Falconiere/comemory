//! Per-candidate fingerprint bundle, the size+mtime skip check, SHA-256
//! identity hashing, and the `source_files` fingerprint upsert — the
//! fast-path plumbing [`crate::document::writer`] calls before (and
//! instead of) running a full extraction.

use std::fs;
use std::time::UNIX_EPOCH;

use rusqlite::Connection;
use sha2::{Digest, Sha256};

use super::writer::UpdateOutcome;
use crate::prelude::*;
use crate::store::sources::{self, SourceFileRow, SourceFileUpsert};

/// Common identity + freshly-taken fingerprint for one candidate,
/// bundled so the write-path helpers in [`crate::document::writer`]
/// don't each carry a long argument list.
pub(super) struct FileStat<'a> {
    pub(super) source_id: &'a str,
    pub(super) relative_path: &'a str,
    pub(super) file_id: &'a str,
    pub(super) size: i64,
    pub(super) mtime: i64,
}

/// Fast pre-read checks: exact-fingerprint skip, then the size ceiling.
/// `Ok(None)` means "proceed to read + hash the content".
pub(super) fn fingerprint_shortcut(
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

/// Upsert a `source_files` row for `stat` with the given
/// `classification`/`status`/`sha256`/`error`. Shared by every
/// `source_files`-only write path (`unsupported`, `too_large`,
/// `error`).
pub(super) fn upsert_stat(
    conn: &Connection,
    stat: &FileStat<'_>,
    classification: &str,
    status: &str,
    sha256: Option<&str>,
    error: Option<&str>,
) -> Result<()> {
    let now = super::writer::iso_now()?;
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

/// Nanoseconds since the Unix epoch for `meta`'s modification time —
/// the `source_files.mtime` fast-fingerprint column (pinned as
/// nanoseconds by the design plan).
pub(super) fn mtime_nanos(meta: &fs::Metadata) -> Result<i64> {
    let modified = meta.modified()?;
    let nanos = modified
        .duration_since(UNIX_EPOCH)
        .map_err(|e| Error::Other(format!("mtime before unix epoch: {e}")))?
        .as_nanos();
    Ok(nanos as i64)
}

/// Full 64-hex-char SHA-256 of `bytes` — the writer's SHA-256 confirm
/// step, and the value stored as both `source_files.sha256` and
/// `documents.revision_hash`.
pub(super) fn content_hash(bytes: &[u8]) -> String {
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
