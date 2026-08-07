//! Reconciles the TOML source registry into the SQLite `source_roots`
//! mirror: every `sources.toml` entry is upserted, and any `source_roots`
//! row absent from the TOML is deleted — the TOML file is authoritative,
//! ephemeral DB state (status, discovery counts) is derived from it.

use std::collections::HashSet;

use rusqlite::Connection;

use crate::prelude::*;
use crate::source::SourceEntry;
use crate::store::memory_row::iso_format;
use crate::store::sources::{self, SourceRootUpsert};

/// Outcome of one [`reconcile`] pass, surfaced by `comemory doctor` and
/// the `index`/`sources`/`unindex` CLI reporting (a later step).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MirrorReport {
    /// Entries newly inserted into `source_roots`.
    pub added: usize,
    /// Existing entries whose row was refreshed.
    pub updated: usize,
    /// Rows deleted because their id no longer appears in `sources.toml`.
    pub removed: usize,
    /// `source_roots` row count after this reconcile.
    pub total: usize,
}

/// Sync `entries` (the live `sources.toml` contents) into `source_roots`.
///
/// Upserts a row per entry — an insert-or-update that preserves
/// `status`/`created_at` on an existing row (see [`sources::upsert`]) —
/// then deletes every `source_roots` row whose id is absent from
/// `entries`. Deletes cascade (`ON DELETE CASCADE`) into
/// `source_files`/`documents`/`document_chunks`.
pub fn reconcile(conn: &Connection, entries: &[SourceEntry]) -> Result<MirrorReport> {
    let existing_ids: HashSet<String> = sources::list(conn)?.into_iter().map(|r| r.id).collect();

    let mut report = MirrorReport::default();
    for entry in entries {
        upsert_entry(conn, entry)?;
        if existing_ids.contains(entry.id.as_str()) {
            report.updated += 1;
        } else {
            report.added += 1;
        }
    }

    let toml_ids: HashSet<&str> = entries.iter().map(|e| e.id.as_str()).collect();
    for id in existing_ids
        .iter()
        .filter(|id| !toml_ids.contains(id.as_str()))
    {
        sources::delete(conn, id)?;
        report.removed += 1;
    }

    report.total = sources::count(conn)?;
    Ok(report)
}

fn upsert_entry(conn: &Connection, entry: &SourceEntry) -> Result<()> {
    let created_at = iso_format(entry.created_at)?;
    let updated_at = iso_format(entry.updated_at)?;
    let canonical_path = entry.canonical_path.to_string_lossy();
    sources::upsert(
        conn,
        SourceRootUpsert {
            id: entry.id.as_str(),
            canonical_path: &canonical_path,
            kind: entry.kind.as_str(),
            repo: entry.repo.as_deref(),
            created_at: &created_at,
            updated_at: &updated_at,
        },
    )
}

#[cfg(test)]
#[path = "tests/mirror.rs"]
mod tests;
