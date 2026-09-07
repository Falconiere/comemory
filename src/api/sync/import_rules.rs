//! Per-entry import rules (spec rules 1–10).

use crate::api::Ctx;
use crate::api::sync::import_state::{
    frontmatter_equal, id_collision, stale_for_cursor, trash_file_exists, trashed_with_hash,
    validate_record,
};
use crate::api::sync::import_write::{log_sync_upsert, patch_frontmatter, write_new_memory};
use crate::api::sync::{ImportEntry, ImportItemResult, ImportStatus, SyncOp};
use crate::cli::delete;
use crate::config::Config;
use crate::memory::MemoryStore;
use crate::memory::id::is_valid_memory_id;
use crate::prelude::*;
use crate::store::sync_log;
use crate::sync::redact;

/// Apply one import entry, returning its disposition.
pub fn apply_entry(
    ctx: &mut Ctx<'_>,
    cursor: i64,
    entry: &ImportEntry,
    author_override: Option<&str>,
    cfg: &Config,
) -> Result<ImportItemResult> {
    let base = ImportItemResult {
        id: entry.id.clone(),
        content_hash: entry.content_hash.clone(),
        status: ImportStatus::Invalid,
        seq: None,
        duplicate_of: None,
        reason: None,
    };
    match entry.op {
        SyncOp::Tombstone => apply_tombstone(ctx, entry, base),
        SyncOp::Restore => apply_restore(ctx, cursor, entry, author_override, base),
        SyncOp::Upsert => apply_upsert(ctx, cursor, entry, author_override, cfg, base),
    }
}

fn apply_tombstone(
    ctx: &mut Ctx<'_>,
    entry: &ImportEntry,
    mut out: ImportItemResult,
) -> Result<ImportItemResult> {
    if !is_valid_memory_id(&entry.id) {
        out.reason = Some("invalid memory id".into());
        return Ok(out);
    }
    let store = MemoryStore::new(ctx.paths.clone());
    let live = store.load(&entry.id).is_ok();
    let trashed = trash_file_exists(ctx.paths, &entry.id);
    if !live && !trashed {
        let conn = ctx.conn()?;
        let tx = conn.transaction()?;
        let seq = sync_log::append(
            &tx,
            SyncOp::Tombstone,
            &entry.id,
            &entry.content_hash,
            &entry.at,
            sync_log::SyncOrigin::Sync,
        )?;
        tx.commit()?;
        out.status = ImportStatus::Deleted;
        out.seq = Some(seq);
        return Ok(out);
    }
    if trashed && !live {
        out.status = ImportStatus::Exists;
        return Ok(out);
    }
    let paths = ctx.paths.clone();
    let conn = ctx.conn()?;
    let (_id, content_hash, _stale) = delete::soft_delete(&paths, conn, &entry.id)?;
    let tx = conn.transaction()?;
    let seq = sync_log::append(
        &tx,
        SyncOp::Tombstone,
        &entry.id,
        &content_hash,
        &entry.at,
        sync_log::SyncOrigin::Sync,
    )?;
    tx.commit()?;
    out.status = ImportStatus::Accepted;
    out.seq = Some(seq);
    Ok(out)
}

fn apply_restore(
    ctx: &mut Ctx<'_>,
    cursor: i64,
    entry: &ImportEntry,
    author_override: Option<&str>,
    mut out: ImportItemResult,
) -> Result<ImportItemResult> {
    let Some(record) = entry.record.as_ref() else {
        out.reason = Some("restore requires record".into());
        return Ok(out);
    };
    if let Some(reason) = validate_record(entry, record) {
        out.reason = Some(reason);
        return Ok(out);
    }
    let conn = ctx.conn()?;
    if stale_for_cursor(conn, &entry.id, cursor)? {
        out.status = ImportStatus::Stale;
        return Ok(out);
    }
    let store = MemoryStore::new(ctx.paths.clone());
    if store.load(&entry.id).is_ok() {
        out.status = ImportStatus::Exists;
        return Ok(out);
    }
    if !trash_file_exists(ctx.paths, &entry.id) {
        out.reason = Some("restore target not in trash".into());
        return Ok(out);
    }
    let _restored = crate::api::restore::run(ctx, &entry.id)?;
    patch_frontmatter(ctx, entry, record, author_override)?;
    let conn = ctx.conn()?;
    let tx = conn.transaction()?;
    let seq = sync_log::append(
        &tx,
        SyncOp::Restore,
        &entry.id,
        &entry.content_hash,
        &entry.at,
        sync_log::SyncOrigin::Sync,
    )?;
    tx.commit()?;
    out.status = ImportStatus::Accepted;
    out.seq = Some(seq);
    Ok(out)
}

fn apply_upsert(
    ctx: &mut Ctx<'_>,
    cursor: i64,
    entry: &ImportEntry,
    author_override: Option<&str>,
    cfg: &Config,
    mut out: ImportItemResult,
) -> Result<ImportItemResult> {
    let Some(record) = entry.record.as_ref() else {
        out.reason = Some("upsert requires record".into());
        return Ok(out);
    };
    if let Some(reason) = validate_record(entry, record) {
        out.reason = Some(reason);
        return Ok(out);
    }
    if let Some(rule) = redact::scan(&record.body) {
        out.status = ImportStatus::SecretDetected;
        out.reason = Some(rule);
        return Ok(out);
    }
    if id_collision(ctx.paths, &entry.id, &entry.content_hash)? {
        out.status = ImportStatus::IdCollision;
        return Ok(out);
    }
    let paths = ctx.paths.clone();
    let conn = ctx.conn()?;
    if stale_for_cursor(conn, &entry.id, cursor)? {
        out.status = ImportStatus::Stale;
        return Ok(out);
    }
    let store = MemoryStore::new(paths.clone());
    if let Ok(rec) = store.load(&entry.id) {
        if frontmatter_equal(&rec, record) {
            out.status = ImportStatus::Exists;
        } else {
            patch_frontmatter(ctx, entry, record, author_override)?;
            let conn = ctx.conn()?;
            let tx = conn.transaction()?;
            let seq = log_sync_upsert(&tx, entry)?;
            tx.commit()?;
            out.status = ImportStatus::Accepted;
            out.seq = Some(seq);
        }
        return Ok(out);
    }
    if trashed_with_hash(conn, &entry.content_hash)? || trash_file_exists(&paths, &entry.id) {
        if trash_file_exists(&paths, &entry.id) {
            let _ = crate::api::restore::run(ctx, &entry.id)?;
            patch_frontmatter(ctx, entry, record, author_override)?;
            let conn = ctx.conn()?;
            let tx = conn.transaction()?;
            let seq = log_sync_upsert(&tx, entry)?;
            tx.commit()?;
            out.status = ImportStatus::Accepted;
            out.seq = Some(seq);
        } else {
            write_new_memory(ctx, entry, record, author_override, cfg, &mut out)?;
        }
        return Ok(out);
    }
    write_new_memory(ctx, entry, record, author_override, cfg, &mut out)?;
    Ok(out)
}
