//! `<data_dir>/replica/erasures.jsonl` — the erasure manifest (#256, B-4):
//! one line per erased entity, append-only, each line carrying the SHA-256 of
//! the line before it, so a truncation or an edit breaks the chain.
//!
//! It lives outside the database on purpose: a restored backup or a `.bak`
//! copied over `comemory.db` takes the database back to before an erase, and
//! this file is how the next [`super::identity::ensure`] knows what to erase
//! again. Every write happens under `identity.lock`, and the
//! [`IdentityGuard`] each writer takes is the proof. A read needs no lock: an
//! append is one write of a whole line, and a torn tail reads as not intact.

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::identity::{IdentityGuard, sync_dir, write_durable};
use crate::prelude::*;
use crate::store::{Connection, erase_rows, replica_redaction, replica_redaction_copies};
use crate::utilities::digest::sha256_hex;

/// The line format's version.
const VERSION: u32 = 1;

/// The `prev` of the first line, which no line precedes.
pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// One erased entity, as the manifest records it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Line {
    /// Line format version.
    pub v: u32,
    /// Entity kind (`memory`, `document`, or an event kind).
    pub kind: String,
    /// Entity key within its kind.
    pub key: String,
    /// Every payload digest the erase blanked.
    pub digests: Vec<String>,
    /// RFC3339 time of the erase.
    pub erased_at: String,
    /// SHA-256 of the previous line's bytes; [`GENESIS`] for the first.
    pub prev: String,
}

/// What one new line says, before it is chained.
#[derive(Debug, Clone, Copy)]
pub struct Entry<'a> {
    /// Entity kind.
    pub kind: &'a str,
    /// Entity key.
    pub key: &'a str,
    /// Every payload digest erased.
    pub digests: &'a [String],
    /// RFC3339 time of the erase.
    pub erased_at: &'a str,
}

impl Entry<'_> {
    /// This entry as a line chained to `prev`.
    fn chained(&self, prev: String) -> Line {
        Line {
            v: VERSION,
            kind: self.kind.to_string(),
            key: self.key.to_string(),
            digests: self.digests.to_vec(),
            erased_at: self.erased_at.to_string(),
            prev,
        }
    }
}

/// What reading the manifest found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifest {
    /// Every line that parses and chains, up to the first that does not.
    pub lines: Vec<Line>,
    /// Whether every line parsed and chained — nothing torn, edited or
    /// reordered.
    pub intact: bool,
}

impl Manifest {
    /// Established: intact, and holding at least the `expected` lines
    /// `identity.json` counts. (A manifest [`read`] found no file for is
    /// never established.)
    #[must_use]
    pub fn established(&self, expected: u64) -> bool {
        self.intact && u64::try_from(self.lines.len()).is_ok_and(|held| held >= expected)
    }
}

/// Read the manifest at `path`; `None` when there is none.
///
/// # Errors
/// Filesystem failures other than a missing file.
pub fn read(path: &Path) -> Result<Option<Manifest>> {
    Ok(bytes_of(path)?.map(|bytes| parse(&bytes)))
}

/// The lines of `bytes` that parse and chain, and whether all of them did.
#[must_use]
pub fn parse(bytes: &[u8]) -> Manifest {
    let mut manifest = Manifest {
        lines: Vec::new(),
        intact: true,
    };
    let mut prev = GENESIS.to_string();
    let mut rest = bytes;
    while !rest.is_empty() {
        // A last line with no newline is one a crash cut short.
        let Some(end) = rest.iter().position(|b| *b == b'\n') else {
            manifest.intact = false;
            break;
        };
        let (raw, tail) = rest.split_at(end);
        rest = tail.get(1..).unwrap_or_default();
        match serde_json::from_slice::<Line>(raw) {
            Ok(line) if line.prev == prev => {
                prev = sha256_hex(raw);
                manifest.lines.push(line);
            }
            _ => {
                manifest.intact = false;
                break;
            }
        }
    }
    manifest
}

/// Append `entry`, chained to the last complete line, and fsync it. A torn
/// tail — a line a crash left half-written, never counted — is cut first, so
/// the new line starts on a line boundary and the chain stays whole.
///
/// # Errors
/// Filesystem and JSON failures.
pub fn append(_held: &IdentityGuard, path: &Path, entry: &Entry<'_>) -> Result<()> {
    let existing = bytes_of(path)?;
    let bytes = existing.as_deref().unwrap_or_default();
    let complete = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    let prev = bytes
        .get(..complete)
        .and_then(|done| done.strip_suffix(b"\n"))
        .map_or_else(|| GENESIS.to_string(), |done| sha256_hex(last_line(done)));
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    if complete < bytes.len() {
        file.set_len(u64::try_from(complete).map_err(|e| Error::Other(e.to_string()))?)?;
    }
    let line = serde_json::to_string(&entry.chained(prev))?;
    file.write_all(format!("{line}\n").as_bytes())?;
    file.sync_all()?;
    if existing.is_none() {
        // A new file's name is durable only once its directory is.
        sync_dir(path)?;
    }
    Ok(())
}

/// Put `bytes` — a manifest the caller verified — in place at `path`
/// durably, replacing whatever is there (`comemory backup merge-erasures`).
///
/// # Errors
/// Filesystem failures.
pub fn install(_held: &IdentityGuard, path: &Path, bytes: &[u8]) -> Result<()> {
    write_durable(path, bytes)
}

/// Write `entries` as a whole new manifest — the first replica read's, from
/// the database — through a fsynced temporary file renamed into place.
///
/// # Errors
/// Filesystem and JSON failures.
pub fn create(_held: &IdentityGuard, path: &Path, entries: &[Entry<'_>]) -> Result<()> {
    let mut text = String::new();
    let mut prev = GENESIS.to_string();
    for entry in entries {
        let line = serde_json::to_string(&entry.chained(prev))?;
        prev = sha256_hex(line.as_bytes());
        text.push_str(&line);
        text.push('\n');
    }
    write_durable(path, text.as_bytes())
}

/// Make every digest `lines` name an erased barrier in `conn`: the payload
/// rows (a digest `conn` never stored gets a bytes-less row, so a later offer
/// of it is refused too), the replay scratch and the complete staged sets —
/// the digest-level half of merging the manifest into a database.
///
/// # Errors
/// Propagates SQLite failures.
pub fn bar_all(conn: &Connection, lines: &[Line], at: &str) -> Result<()> {
    let mut digests = Vec::new();
    for line in lines {
        replica_redaction::bar(conn, &line.digests, &line.kind, at)?;
        digests.extend(line.digests.iter().cloned());
    }
    replica_redaction_copies::clear_replay_of(conn, &digests)?;
    erase_rows::erase_staged_of(conn, &digests)?;
    Ok(())
}

/// The bytes after the last newline in `done` — its last line.
fn last_line(done: &[u8]) -> &[u8] {
    let start = done.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    done.get(start..).unwrap_or_default()
}

/// The file's bytes; `None` when it does not exist.
fn bytes_of(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
#[path = "tests/erasure_manifest.rs"]
mod tests;
