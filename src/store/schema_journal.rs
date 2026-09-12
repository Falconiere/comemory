//! The two journal operations `examples/migrations.rs` exposes as
//! `just migration-journal` and `just migration-adopt`, kept in the crate so
//! they are tested against the shipped `migrations/` files rather than only
//! exercised by hand: [`journal_file`] records a hand-written migration in
//! `_journal.json` (name + SHA-256, in order), and [`adopt`] restates the
//! newest journal entry's snapshot from [`super::schema::registry`].
//!
//! `generate` is not here — it is toolu-orm's `run_generate`, a
//! `toolu-orm-cli` dev-dependency the release binary never links. Neither
//! function touches a database; `store::migrate` remains the only apply path.

use std::path::{Path, PathBuf};

use toolu_orm::core::journal::{Journal, compute_hash};
use toolu_orm::core::snapshot::Snapshot;

use super::schema::registry;
use crate::prelude::*;

/// What [`journal_file`] recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Journaled {
    /// The entry name — the migration file's basename, e.g. `0017_add_x.sql`.
    pub name: String,
    /// `sha256:<hex>` of the file bytes, as `_journal.json` now carries it.
    pub hash: String,
}

/// `<dir>/_journal.json` as the `&str` toolu-orm's file APIs take.
fn journal_path(dir: &Path) -> Result<String> {
    Ok(format!("{}/_journal.json", dir_str(dir)?))
}

fn dir_str(dir: &Path) -> Result<&str> {
    dir.to_str()
        .ok_or_else(|| Error::Other(format!("{} is not valid UTF-8", dir.display())))
}

/// Append `migration` (a file inside or outside `dir` — only its basename
/// and bytes matter) to `<dir>/_journal.json`, hashing the bytes exactly as
/// `run_generate` would. A name already journaled is refused with
/// [`Error::Usage`] and nothing is written: `run_generate` numbers the next
/// file from `max(NNNN) + 1`, so a duplicate entry would be invisible until
/// it collided.
pub fn journal_file(dir: &Path, migration: &Path) -> Result<Journaled> {
    let name = migration
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| Error::Other(format!("{}: no file name", migration.display())))?
        .to_owned();
    let content = std::fs::read_to_string(migration)
        .map_err(|e| Error::Other(format!("{}: {e}", migration.display())))?;
    let path = journal_path(dir)?;
    let mut journal =
        Journal::read_from_path(&path).map_err(|e| Error::Other(format!("journal: {e}")))?;
    if journal.entries.iter().any(|entry| entry.name == name) {
        return Err(Error::Usage(format!("{name} is already journaled")));
    }
    let hash = compute_hash(&content);
    journal.add_entry(&name, &hash);
    journal
        .write_to_path(&path)
        .map_err(|e| Error::Other(format!("journal: {e}")))?;
    Ok(Journaled { name, hash })
}

/// Rewrite the newest journal entry's `<base>.snapshot.json` in `dir` from
/// [`registry`], returning the file written. An existing snapshot keeps its
/// `id` / `prev_id` — this is a restatement, not a new generation — so a
/// second run writes byte-identical content. An empty journal is
/// [`Error::Usage`]: there is no entry to attach the snapshot to.
pub fn adopt(dir: &Path) -> Result<PathBuf> {
    let journal = Journal::read_from_path(&journal_path(dir)?)
        .map_err(|e| Error::Other(format!("journal: {e}")))?;
    let snapshot_name = journal.latest_snapshot_name_owned().ok_or_else(|| {
        Error::Usage("journal is empty; journal at least one migration first".to_owned())
    })?;
    let snapshot_file = dir.join(&snapshot_name);
    let snapshot_str = snapshot_file
        .to_str()
        .ok_or_else(|| Error::Other(format!("{} is not valid UTF-8", snapshot_file.display())))?;
    let mut fresh = Snapshot::from_registry(&registry());
    if snapshot_file.exists() {
        let existing = Snapshot::read_from_path(snapshot_str)
            .map_err(|e| Error::Other(format!("snapshot: {e}")))?;
        fresh.id = existing.id;
        fresh.prev_id = existing.prev_id;
    }
    fresh
        .write_to_path(snapshot_str)
        .map_err(|e| Error::Other(format!("snapshot: {e}")))?;
    Ok(snapshot_file)
}

#[cfg(test)]
#[path = "tests/schema_journal.rs"]
mod tests;
