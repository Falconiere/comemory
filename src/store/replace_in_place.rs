//! Replace a live database's content in place through SQLite's online backup
//! API (#256, B-3) — how `comemory rebuild` installs a freshly built mirror.
//!
//! Renaming a new file over `comemory.db` left every already-open connection
//! (the server's, an MCP session's) reading the unlinked old inode, and a stale
//! connection closing last could delete the new file's WAL by name. Copying
//! the pages into the destination through a normal write transaction keeps one
//! file: every connection already open on it reads the new content on its next
//! query, no sidecar is renamed or removed, and a copy that fails part-way is
//! rolled back like any other transaction.

use std::ffi::c_int;
use std::io::Read as _;
use std::path::Path;
use std::time::Duration;

use rusqlite::backup::{Backup, Progress, StepResult};

use crate::prelude::*;
use crate::store::Connection;

/// Open `path` as the destination of [`replace_in_place`]: a plain connection
/// that waits up to `busy_wait` for another writer — no PRAGMA, no migration,
/// since its whole content is about to be replaced.
pub fn open_destination(path: &Path, busy_wait: Duration) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(busy_wait)?;
    Ok(conn)
}

/// Replace `dest`'s main database with the database file at `source`, in
/// place, in one write transaction.
///
/// # Errors
/// [`Error::Busy`] when another writer holds the destination past its busy
/// timeout; [`Error::Other`] naming both sizes when the page sizes differ.
/// Either way — and on any SQLite failure — `dest` is left unchanged.
pub fn replace_in_place(dest: &mut Connection, source: &Path) -> Result<()> {
    copy_pages(dest, source, -1, &mut |_| {})
}

/// [`replace_in_place`] copying `pages_per_step` pages per backup step and
/// calling `between_steps` after each step that left pages to copy — the
/// destination stays write-locked throughout, which is what a test observes.
fn copy_pages(
    dest: &mut Connection,
    source: &Path,
    pages_per_step: c_int,
    between_steps: &mut dyn FnMut(Progress),
) -> Result<()> {
    let src = Connection::open(source)?;
    refuse_page_size_mismatch(&src, dest)?;
    // Dropping the handle finishes the backup: a copy that stops before
    // `Done` rolls the destination's write transaction back.
    let backup = Backup::new(&src, dest)?;
    loop {
        match backup.step(pages_per_step)? {
            StepResult::Done => return Ok(()),
            StepResult::More => between_steps(backup.progress()),
            StepResult::Busy | StepResult::Locked => {
                return Err(Error::Busy(
                    "the database is locked by another writer".into(),
                ));
            }
            other => {
                return Err(Error::Other(format!(
                    "unexpected backup step result: {other:?}"
                )));
            }
        }
    }
}

/// The page size recorded in the header of the SQLite file at `path`, read
/// from the file itself — nothing is opened, created or migrated, so a
/// restore can refuse a snapshot it could not replace the live file with
/// before it touches anything.
///
/// # Errors
/// [`Error::Other`] when `path` is not a SQLite database; filesystem
/// failures.
pub fn page_size(path: &Path) -> Result<u32> {
    let mut header = [0u8; 18];
    std::fs::File::open(path)?.read_exact(&mut header)?;
    let (magic, size) = header.split_at(16);
    if magic != b"SQLite format 3\0" {
        return Err(Error::Other(format!(
            "{} is not a SQLite database",
            path.display()
        )));
    }
    // The header stores 65536 as 1: it does not fit in two bytes.
    Ok(match u16::from_be_bytes([size[0], size[1]]) {
        1 => 65_536,
        raw => u32::from(raw),
    })
}

/// Refuse a copy between different page sizes up front. SQLite refuses it
/// too when the destination is in WAL mode, but only as "attempt to write a
/// readonly database", which names neither size.
fn refuse_page_size_mismatch(source: &Connection, dest: &Connection) -> Result<()> {
    let page_size = |conn: &Connection| -> Result<i64> {
        Ok(conn.query_row("PRAGMA page_size", [], |r| r.get(0))?)
    };
    let (from, to) = (page_size(source)?, page_size(dest)?);
    if from == to {
        return Ok(());
    }
    Err(Error::Other(format!(
        "cannot replace the database in place: the source uses {from}-byte pages \
         and the destination {to}-byte pages"
    )))
}

#[cfg(test)]
#[path = "tests/replace_in_place.rs"]
mod tests;
