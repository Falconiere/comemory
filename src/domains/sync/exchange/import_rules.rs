//! Per-entry import rules (spec rules 1–10).

use crate::config::Config;
use crate::domains::memories::MemoryStore;
use crate::domains::memories::delete;
use crate::domains::memories::id::is_valid_memory_id;
use crate::domains::memories::journal;
use crate::domains::sync::exchange::import_state::{
    frontmatter_equal, id_collision, stale_for_cursor, trash_file_exists, trashed_with_hash,
    validate_record,
};
use crate::domains::sync::exchange::import_write::{patch_frontmatter, write_new_memory};
use crate::domains::sync::exchange::{
    ImportEntry, ImportItemResult, ImportStatus, SyncOp, SyncRecord,
};
use crate::domains::sync::redact;
use crate::prelude::*;
use crate::store::replica_journal::{ReplicaOp, ReplicaOrigin};
use crate::utilities::context::Ctx;

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
        let seq = journal::record_tombstone(
            &tx,
            &entry.id,
            &entry.content_hash,
            None,
            &entry.at,
            ReplicaOrigin::Sync,
        )?
        .legacy_seq;
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
    // Journalled inside the delete's own transaction: a crash between the two
    // would otherwise leave the memory deleted with nothing recording it, and
    // the next pull would bring it back.
    let removed = delete::soft_delete(
        &paths,
        conn,
        &entry.id,
        Some(ReplicaOrigin::Sync),
        Some(&entry.at),
    )?;
    let seq = removed
        .journalled
        .ok_or_else(|| Error::Other("import tombstone was not journalled".to_string()))?
        .legacy_seq;
    out.status = ImportStatus::Accepted;
    out.seq = Some(seq);
    Ok(out)
}

/// The checks every payload-carrying rule makes before it looks at state: the
/// entry carries a record, and that record is internally consistent.
///
/// Returns `None` once `out` describes the refusal, so a caller's guard chain
/// is one line instead of a copy of this one.
fn precheck_payload<'a>(
    entry: &'a ImportEntry,
    op: &str,
    out: &mut ImportItemResult,
) -> Option<&'a SyncRecord> {
    let Some(record) = entry.record.as_ref() else {
        out.reason = Some(format!("{op} requires record"));
        return None;
    };
    if let Some(reason) = validate_record(entry, record) {
        out.reason = Some(reason);
        return None;
    }
    Some(record)
}

/// [`precheck_payload`] plus the cursor check, for the rules that also refuse
/// an entry pushed without having seen a newer tombstone.
///
/// # Errors
/// Propagates SQLite failures from the tombstone lookup.
fn precheck<'a>(
    ctx: &mut Ctx<'_>,
    cursor: i64,
    entry: &'a ImportEntry,
    op: &str,
    out: &mut ImportItemResult,
) -> Result<Option<&'a SyncRecord>> {
    let Some(record) = precheck_payload(entry, op, out) else {
        return Ok(None);
    };
    if stale_for_cursor(ctx.conn()?, &entry.id, cursor)? {
        out.status = ImportStatus::Stale;
        return Ok(None);
    }
    Ok(Some(record))
}

fn apply_restore(
    ctx: &mut Ctx<'_>,
    cursor: i64,
    entry: &ImportEntry,
    author_override: Option<&str>,
    mut out: ImportItemResult,
) -> Result<ImportItemResult> {
    let Some(record) = precheck(ctx, cursor, entry, "restore", &mut out)? else {
        return Ok(out);
    };
    let store = MemoryStore::new(ctx.paths.clone());
    if store.load(&entry.id).is_ok() {
        out.status = ImportStatus::Exists;
        return Ok(out);
    }
    if !trash_file_exists(ctx.paths, &entry.id) {
        out.reason = Some("restore target not in trash".into());
        return Ok(out);
    }
    crate::domains::memories::restore::restore_one(ctx, &entry.id)?;
    accept_patch(
        ctx,
        entry,
        record,
        author_override,
        ReplicaOp::Restore,
        &mut out,
    )?;
    Ok(out)
}

/// Patch the live markdown from the wire record and journal the result, then
/// stamp the disposition.
///
/// Shared by the restore and upsert rules: once a rule has decided the entry
/// may be applied, what follows is the same write in both, and keeping one
/// copy is what stops them from journalling differently.
fn accept_patch(
    ctx: &mut Ctx<'_>,
    entry: &ImportEntry,
    record: &SyncRecord,
    author_override: Option<&str>,
    op: ReplicaOp,
    out: &mut ImportItemResult,
) -> Result<()> {
    let patched = patch_frontmatter(ctx, entry, record, author_override)?;
    let conn = ctx.conn()?;
    let tx = conn.transaction()?;
    let seq = journal::record_write(
        &tx,
        op,
        &patched.frontmatter,
        &patched.body,
        &entry.at,
        ReplicaOrigin::Sync,
    )?
    .legacy_seq;
    tx.commit()?;
    out.status = ImportStatus::Accepted;
    out.seq = Some(seq);
    Ok(())
}

fn apply_upsert(
    ctx: &mut Ctx<'_>,
    cursor: i64,
    entry: &ImportEntry,
    author_override: Option<&str>,
    cfg: &Config,
    mut out: ImportItemResult,
) -> Result<ImportItemResult> {
    let Some(record) = precheck_payload(entry, "upsert", &mut out) else {
        return Ok(out);
    };
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
            accept_patch(
                ctx,
                entry,
                record,
                author_override,
                ReplicaOp::Upsert,
                &mut out,
            )?;
        }
        return Ok(out);
    }
    let in_trash = trash_file_exists(&paths, &entry.id);
    if trashed_with_hash(conn, &entry.content_hash)? || in_trash {
        if in_trash {
            crate::domains::memories::restore::restore_one(ctx, &entry.id)?;
            accept_patch(
                ctx,
                entry,
                record,
                author_override,
                ReplicaOp::Upsert,
                &mut out,
            )?;
        } else {
            write_new_memory(ctx, entry, record, author_override, cfg, &mut out)?;
        }
        return Ok(out);
    }
    write_new_memory(ctx, entry, record, author_override, cfg, &mut out)?;
    Ok(out)
}
