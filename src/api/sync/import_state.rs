//! Import validation and memory-state probes (rules 1–2, 6).

use rusqlite::{Connection, OptionalExtension};

use crate::api::sync::{ImportEntry, SyncRecord};
use crate::memory::MemoryStore;
use crate::memory::frontmatter::Frontmatter;
use crate::memory::id::{is_valid_memory_id, memory_id, sha256_hex};
use crate::prelude::*;
use crate::store::sync_log;

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

/// Rule 2 — live or trashed id bound to a different content hash.
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
    if let Ok(rec) = store.load(id) {
        return Ok(rec.frontmatter.content_hash != content_hash);
    }
    if let Some(path) = trash_path(paths, id) {
        let raw = std::fs::read_to_string(path)?;
        let (fm, _) = Frontmatter::split(&raw)?;
        return Ok(fm.content_hash != content_hash);
    }
    Ok(false)
}

/// Rule 6 — pusher has not yet observed a newer tombstone.
pub(crate) fn stale_for_cursor(conn: &Connection, memory_id: &str, cursor: i64) -> Result<bool> {
    Ok(sync_log::latest_tombstone_seq(conn, memory_id)?.is_some_and(|seq| seq > cursor))
}

/// Whether a soft-deleted mirror row carries `content_hash`.
pub(crate) fn trashed_with_hash(conn: &Connection, content_hash: &str) -> Result<bool> {
    conn.query_row(
        "SELECT 1 FROM memories WHERE content_hash = ?1 AND deleted_at IS NOT NULL",
        [content_hash],
        |r| r.get::<_, i64>(0),
    )
    .optional()
    .map(|row| row.is_some())
    .map_err(Into::into)
}

/// True when `.trash/` holds a markdown file for `id`.
pub(crate) fn trash_file_exists(paths: &crate::config::Paths, id: &str) -> bool {
    trash_path(paths, id).is_some()
}

fn trash_path(paths: &crate::config::Paths, id: &str) -> Option<std::path::PathBuf> {
    let prefix = format!("{id}-");
    std::fs::read_dir(paths.trash_dir())
        .ok()?
        .flatten()
        .find(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
        .map(|entry| entry.path())
}
