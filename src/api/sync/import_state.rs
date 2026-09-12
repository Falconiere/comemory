//! Import validation and memory-state probes (rules 1–2, 6).

use crate::api::sync::{ImportEntry, SyncRecord};
use crate::memory::MemoryStore;
use crate::memory::id::{is_valid_memory_id, memory_id, sha256_hex};
use crate::prelude::*;
use crate::store::{Connection, memory_purge, sync_log};

/// Rule 1 — schema/hash/id validation for upsert/restore payloads.
pub(crate) fn validate_record(entry: &ImportEntry, record: &SyncRecord) -> Option<String> {
    if record.frontmatter.schema != 1 {
        return Some(format!("unsupported schema {}", record.frontmatter.schema));
    }
    if entry.id != record.frontmatter.id {
        return Some("entry id disagrees with frontmatter.id".into());
    }
    if entry.content_hash != record.frontmatter.content_hash {
        return Some("entry content_hash disagrees with frontmatter".into());
    }
    if !is_valid_memory_id(&entry.id) {
        return Some("invalid memory id".into());
    }
    if entry.id != memory_id(&record.body) {
        return Some("id does not match body hash".into());
    }
    if entry.content_hash != sha256_hex(record.body.trim_end().as_bytes()) {
        return Some("content_hash does not match body".into());
    }
    if !(1..=5).contains(&record.frontmatter.quality) {
        return Some("quality out of range".into());
    }
    None
}

/// Rule 8 — whether live frontmatter already matches the wire payload.
pub(crate) fn frontmatter_equal(rec: &crate::memory::MemoryRecord, wire: &SyncRecord) -> bool {
    let fm = &rec.frontmatter;
    let w = &wire.frontmatter;
    fm.kind == w.kind
        && fm.repo == w.repo
        && fm.tags == w.tags
        && fm.quality == w.quality
        && fm.content_hash == w.content_hash
        && fm.references == w.references
        && fm.relations == w.relations
}

/// Rule 2 — live or trashed id bound to a different content hash: the
/// same 32-bit collision `api::save` refuses, answered by the same
/// `MemoryStore::prior` lookup so the two rules cannot drift. One
/// deliberate difference: a local copy that exists but cannot be parsed is
/// logged and treated as *no* collision, so a pull can repair it —
/// `api::save` is stricter and refuses to overwrite what it cannot read.
pub(crate) fn id_collision(
    paths: &crate::config::Paths,
    id: &str,
    content_hash: &str,
) -> Result<bool> {
    id_collision_inner(paths, id, content_hash)
}

/// Test-only re-export of [`id_collision`].
#[cfg(test)]
pub(crate) fn id_collision_for_test(
    paths: &crate::config::Paths,
    id: &str,
    content_hash: &str,
) -> Result<bool> {
    id_collision_inner(paths, id, content_hash)
}

fn id_collision_inner(paths: &crate::config::Paths, id: &str, content_hash: &str) -> Result<bool> {
    let store = MemoryStore::new(paths.clone());
    match store.prior(id) {
        Ok(prior) => Ok(prior.is_some_and(|prior| prior.collides_with(content_hash))),
        Err(Error::Io(e)) => Err(Error::Io(e)),
        Err(e) => {
            tracing::warn!(
                memory_id = id,
                error = %e,
                "unreadable local copy; the import treats it as no collision"
            );
            Ok(false)
        }
    }
}

/// Rule 6 — pusher has not yet observed a newer tombstone.
pub(crate) fn stale_for_cursor(conn: &Connection, memory_id: &str, cursor: i64) -> Result<bool> {
    Ok(sync_log::latest_tombstone_seq(conn, memory_id)?.is_some_and(|seq| seq > cursor))
}

/// Whether a soft-deleted mirror row carries `content_hash`.
pub(crate) fn trashed_with_hash(conn: &Connection, content_hash: &str) -> Result<bool> {
    memory_purge::trashed_with_hash(conn, content_hash)
}

/// True when `.trash/` holds a markdown file for `id` — the store's own
/// trash lookup, so this and the save-time purge agree on what counts.
pub(crate) fn trash_file_exists(paths: &crate::config::Paths, id: &str) -> bool {
    MemoryStore::new(paths.clone()).trash_entry(id).is_some()
}
