//! Read and write indexed source files for the `comemory serve` editor.
//!
//! Reads return the file contents plus a freshly-computed git blob OID (the
//! same hash git would assign the working-tree bytes). Writes enforce a size
//! cap, an editable-extension allowlist, and `If-Match` optimistic concurrency
//! keyed on that blob OID: if the on-disk bytes changed since the client's
//! `GET` (e.g. the user saved from their real editor), the write is refused as
//! a [`WriteOutcome::Conflict`] rather than clobbering the newer content. The
//! write itself is atomic (temp file in the same directory + rename).

use std::path::Path;

use git2::ObjectType;
use serde::Serialize;

use crate::ast::languages;
use crate::prelude::*;

/// Maximum editable file size (bytes). Source files are far smaller; the cap
/// stops the editor from trying to load or persist a huge blob. Exposed so the
/// router can layer a matching `DefaultBodyLimit`, keeping axum's body-limit
/// rejection and this in-handler check on the same threshold.
pub(crate) const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;

/// A file's contents plus the metadata the editor needs.
#[derive(Debug, Serialize)]
pub struct FileView {
    /// Repo-relative display path (the `<path>` from the node id).
    pub path: String,
    /// Canonical language name (`rust`, `python`, …) or `"text"`.
    pub lang: String,
    /// UTF-8 file contents.
    pub contents: String,
    /// git blob OID of the current on-disk bytes (the `If-Match` token).
    pub blob_oid: String,
}

/// Result of a write attempt.
#[derive(Debug)]
pub enum WriteOutcome {
    /// The file was written; carries the new blob OID.
    Written { blob_oid: String },
    /// The on-disk bytes no longer match the client's `If-Match`; carries the
    /// current blob OID so the client can reload before retrying.
    Conflict { current_oid: String },
}

/// git blob OID of `bytes` — the hash git would store, computed without
/// touching the object database (no repo required).
fn blob_oid_of(bytes: &[u8]) -> Result<String> {
    Ok(git2::Oid::hash_object(ObjectType::Blob, bytes)?.to_string())
}

/// Read `abs` as UTF-8 text, attaching the `display` path and detected
/// language. Rejects oversized files and non-UTF-8 (binary) content.
pub fn read_file(abs: &Path, display: &str) -> Result<FileView> {
    let meta = std::fs::metadata(abs).map_err(Error::Io)?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(Error::BadRequest(format!(
            "file too large to edit ({} bytes > {MAX_FILE_BYTES})",
            meta.len()
        )));
    }
    let bytes = std::fs::read(abs).map_err(Error::Io)?;
    let contents = String::from_utf8(bytes)
        .map_err(|_| Error::BadRequest("file is not valid UTF-8 text".into()))?;
    let lang = languages::detect(abs)
        .map(|l| l.as_str().to_string())
        .unwrap_or_else(|| "text".into());
    let blob_oid = blob_oid_of(contents.as_bytes())?;
    Ok(FileView {
        path: display.to_string(),
        lang,
        contents,
        blob_oid,
    })
}

/// Write `contents` to `abs`, enforcing the size cap, the editable-extension
/// allowlist (only languages comemory can index), and — when `if_match` is
/// supplied — `If-Match` optimistic concurrency against the current on-disk
/// blob OID. The write goes to a temp file in the same directory and is then
/// renamed over `abs`, so a crash mid-write cannot leave a truncated file.
pub fn write_file(abs: &Path, contents: &str, if_match: Option<&str>) -> Result<WriteOutcome> {
    if contents.len() as u64 > MAX_FILE_BYTES {
        return Err(Error::BadRequest(format!(
            "content too large ({} bytes > {MAX_FILE_BYTES})",
            contents.len()
        )));
    }
    if languages::detect(abs).is_none() {
        return Err(Error::Forbidden(
            "file type is not editable (unsupported language)".into(),
        ));
    }
    // Optimistic concurrency: compare the client's expected OID with the
    // current on-disk bytes. A missing file with an If-Match is also a
    // conflict (the client expected a specific version that is gone).
    if let Some(expected) = if_match {
        let current = match std::fs::read(abs) {
            Ok(bytes) => Some(blob_oid_of(&bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(Error::Io(e)),
        };
        if current.as_deref() != Some(expected) {
            return Ok(WriteOutcome::Conflict {
                current_oid: current.unwrap_or_default(),
            });
        }
    }
    let parent = abs
        .parent()
        .ok_or_else(|| Error::BadRequest("target has no parent directory".into()))?;
    let file_name = abs
        .file_name()
        .ok_or_else(|| Error::BadRequest("target has no file name".into()))?
        .to_string_lossy();
    let tmp = parent.join(format!(".{file_name}.comemory.tmp"));
    std::fs::write(&tmp, contents.as_bytes()).map_err(Error::Io)?;
    // Carry the original file's permissions onto the replacement so a save
    // never silently relaxes a 0600 file or drops the +x bit on a script.
    // Best-effort: a permissions copy failure must not sink an otherwise-good
    // write (the rename below is what actually matters).
    if let Ok(meta) = std::fs::metadata(abs) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    if let Err(e) = std::fs::rename(&tmp, abs) {
        // Best-effort cleanup so a failed rename does not litter temp files.
        let _ = std::fs::remove_file(&tmp);
        return Err(Error::Io(e));
    }
    Ok(WriteOutcome::Written {
        blob_oid: blob_oid_of(contents.as_bytes())?,
    })
}
