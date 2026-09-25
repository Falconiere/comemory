//! Journal seeding for memories trashed before the journal existed.
//!
//! A rebuild replays only top-level `memories/*.md` into the mirror
//! (`domains::memories::store`), so a previously trashed memory has no
//! mirror row at all after one — `.trash/` on disk is the only source both a
//! resume cursor and the tombstone's provenance time can come from. Files
//! there are listed and sorted by name, which is monotonic and stable across
//! runs, and each is journalled as a tombstone from its own mtime: the
//! payload-free revision a later restore references.

use crate::domains::memories::Frontmatter;
use crate::domains::sync::replica::bootstrap::{Progress, STATE_COMPLETE};
use crate::prelude::*;
use crate::store::connection::write_transaction;
use crate::store::replica_journal::ReplicaOrigin;
use crate::store::{replica_read, schema_meta};
use crate::utilities::context::Ctx;

/// Trash entries scanned per call — the same batch size
/// [`super::bootstrap`] uses.
const BATCH: usize = 200;

/// `schema_meta` key holding `pending`, `seeding` or `complete`.
pub const STATE_KEY: &str = "replica_seed_trash_state";

/// `schema_meta` key holding the last scanned `.trash/` filename.
pub const THROUGH_KEY: &str = "replica_seed_trash_through";

/// Read the stored progress.
///
/// # Errors
/// Propagates SQLite failures.
pub fn progress(ctx: &mut Ctx<'_>) -> Result<Progress> {
    let conn = ctx.conn()?;
    let state = schema_meta::get(conn, STATE_KEY)?.unwrap_or_else(|| "pending".to_string());
    let through = schema_meta::get(conn, THROUGH_KEY)?.unwrap_or_default();
    Ok(Progress { state, through })
}

/// Seed up to [`BATCH`] unjournalled trashed memories and return the new
/// progress. Idempotent: a memory already carrying a revision is skipped.
///
/// # Errors
/// Propagates markdown and SQLite failures.
pub fn advance(ctx: &mut Ctx<'_>) -> Result<Progress> {
    let current = progress(ctx)?;
    if current.complete() {
        return Ok(current);
    }
    let files = trash_files_after(&ctx.paths.trash_dir(), &current.through, BATCH);
    let Some(last) = files.last().cloned() else {
        return mark(ctx, STATE_COMPLETE, &current.through);
    };
    let finished = files.len() < BATCH;
    for file in &files {
        seed_one(ctx, file)?;
    }
    mark(
        ctx,
        if finished { STATE_COMPLETE } else { "seeding" },
        &last,
    )
}

/// `.trash/*.md` filenames greater than `after`, ascending, capped at
/// `limit`. A missing directory reads as "nothing to seed" rather than
/// aborting the walk — `gc` treats a missing trash directory the same way.
fn trash_files_after(dir: &std::path::Path, after: &str, limit: usize) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(std::result::Result::ok)
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| {
            std::path::Path::new(n)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
                && n.as_str() > after
        })
        .collect();
    names.sort();
    names.truncate(limit);
    names
}

/// Load one trash file, journal its tombstone unless it already has a
/// revision, stamped with the file's own mtime — the moment the delete moved
/// it, which survives a rebuild even though the mirror row does not.
fn seed_one(ctx: &mut Ctx<'_>, file_name: &str) -> Result<()> {
    let path = ctx.paths.trash_dir().join(file_name);
    // A file removed between listing and reading is `gc`'s business, not a
    // reason to stall every other trashed memory.
    let (Ok(raw), Ok(meta)) = (std::fs::read_to_string(&path), std::fs::metadata(&path)) else {
        return Ok(());
    };
    let (fm, _body) = Frontmatter::split(&raw)?;
    {
        let conn = ctx.conn()?;
        if replica_read::revision(conn, "memory", &fm.id)?.is_some() {
            return Ok(());
        }
    }
    let Ok(modified) = meta.modified() else {
        return Ok(());
    };
    let at = crate::store::memory_row::iso_format(time::OffsetDateTime::from(modified))?;
    journal_tombstone(ctx, &fm, &at)
}

/// Journal one trashed memory's tombstone, inside its own transaction.
fn journal_tombstone(ctx: &mut Ctx<'_>, fm: &Frontmatter, at: &str) -> Result<()> {
    let repository = (!fm.repo.is_empty()).then_some(fm.repo.as_str());
    let conn = ctx.conn()?;
    let tx = write_transaction(conn)?;
    if replica_read::revision(&tx, "memory", &fm.id)?.is_some() {
        // A restore, or a concurrent seed, journalled this id first.
        return Ok(());
    }
    crate::domains::memories::journal::record_tombstone(
        &tx,
        &fm.id,
        &fm.content_hash,
        repository,
        at,
        ReplicaOrigin::Local,
        None,
    )?;
    tx.commit()?;
    Ok(())
}

/// Store the progress markers.
fn mark(ctx: &mut Ctx<'_>, state: &str, through: &str) -> Result<Progress> {
    let conn = ctx.conn()?;
    schema_meta::upsert(conn, STATE_KEY, state)?;
    schema_meta::upsert(conn, THROUGH_KEY, through)?;
    Ok(Progress {
        state: state.to_string(),
        through: through.to_string(),
    })
}

#[cfg(test)]
#[path = "tests/seed_trash.rs"]
mod tests;
