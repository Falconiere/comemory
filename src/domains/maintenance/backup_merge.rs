//! Merging an erasure manifest into a database (#256, B-4): what a restore
//! runs against its staged copy, and `comemory backup merge-erasures FILE`
//! runs against the live one once it is given the manifest a restore could
//! not find.
//!
//! Each entry naming a memory or a document is erased in full through
//! [`erase::apply_locked`] — mirror rows, FTS, markdown, payload bytes, a
//! tombstone when the entity is live — without appending to the manifest,
//! which already holds it. Then every digest every entry names is barred,
//! so an entity the database never held is refused when a peer offers it.

use std::path::Path;

use super::Merged;
use crate::config::{Config, Paths};
use crate::domains::documents::replica_payload::DOCUMENT_ENTITY_KIND;
use crate::domains::maintenance::erase::{self, Target};
use crate::domains::memories::replica_payload::MEMORY_ENTITY_KIND;
use crate::domains::memories::save_lock::{self, SaveGuard};
use crate::domains::sync::replica::erasure_manifest::{self, Line};
use crate::domains::sync::replica::identity::{self, MANIFEST_FILE};
use crate::domains::sync::replica::restore_state;
use crate::prelude::*;
use crate::store::erase_rows::{self, FtsIndex};
use crate::store::{Connection, connection};

/// Merge `lines` into the database behind `conn` and the markdown tree at
/// `memories_dir`; returns how many entries erased an entity. The guard is
/// the proof the caller holds `memory-save.lock`.
///
/// # Errors
/// SQLite and filesystem failures. An entity the database never held is not
/// an error: its digests are still barred.
pub(super) fn lines_into(
    guard: &SaveGuard,
    memories_dir: &Path,
    conn: &mut Connection,
    lines: &[Line],
) -> Result<usize> {
    let mut erased = 0;
    let mut touched: Vec<FtsIndex> = Vec::new();
    for line in lines {
        let target = match line.kind.as_str() {
            MEMORY_ENTITY_KIND => Target::Memory(line.key.clone()),
            DOCUMENT_ENTITY_KIND => Target::Document(line.key.clone()),
            // Events and orphaned digests have no entity to erase: barring
            // their digests below is the whole merge.
            _ => continue,
        };
        match erase::apply_locked(guard, None, memories_dir, conn, &target) {
            Ok(applied) => {
                erased += 1;
                for index in applied.touched {
                    if !touched.contains(&index) {
                        touched.push(index);
                    }
                }
            }
            Err(Error::NotFound(_)) => {}
            Err(e) => return Err(e),
        }
    }
    let at = crate::store::memory_row::iso_format(time::OffsetDateTime::now_utc())?;
    erase_rows::with_secure_delete(conn, |conn| {
        let tx = connection::write_transaction(conn)?;
        erasure_manifest::bar_all(&tx, lines, &at)?;
        tx.commit()?;
        Ok(())
    })?;
    let _wal_truncated = erase::settle(conn, &touched)?;
    Ok(erased)
}

/// The merge behind [`super::merge_erasures`].
pub(super) fn run(paths: &Paths, cfg: &Config, file: &Path) -> Result<Merged> {
    let bytes = read_ready(paths, file)?;
    let manifest = erasure_manifest::parse(&bytes);
    let wait = cfg.sync.pause_wait_duration()?;
    let guard = save_lock::acquire_within(paths, wait)?;
    let held = identity::lock(paths, wait)?;
    let mut conn = connection::open(paths.db_path())?;
    let (identity, _) = identity::establish(&held, paths, &conn)?;
    if !manifest.established(identity.erasures) {
        return Err(Error::Conflict(format!(
            "{} is not an established erasure manifest: {} verified line(s), intact: {}, \
             and the identity counts {}",
            file.display(),
            manifest.lines.len(),
            manifest.intact,
            identity.erasures
        )));
    }
    let target = identity::file(paths, MANIFEST_FILE);
    erasure_manifest::install(&held, &target, &bytes)?;
    let entities_erased = lines_into(&guard, &paths.memories_dir(), &mut conn, &manifest.lines)?;
    let erasures = identity.erasures.max(identity::count(manifest.lines.len()));
    let cleared = verified(&mut conn, erasures)?;
    Ok(Merged {
        manifest: target.to_string_lossy().into_owned(),
        lines: manifest.lines.len(),
        entities_erased,
        cleared,
    })
}

/// The bytes of `file`, once the data directory can take a merge: a database
/// exists and no restore is mid-swap.
fn read_ready(paths: &Paths, file: &Path) -> Result<Vec<u8>> {
    let db = paths.db_path();
    if !db.try_exists()? {
        return Err(Error::Unavailable(format!(
            "no database to merge into at {}",
            db.display()
        )));
    }
    if restore_state::pending_path(paths).try_exists()? {
        return Err(Error::Conflict(
            "a restore is still pending; rerun `comemory backup restore <dir> --confirm` first"
                .to_string(),
        ));
    }
    match std::fs::read(file) {
        Ok(bytes) => Ok(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(Error::Usage(format!("{} does not exist", file.display())))
        }
        Err(e) => Err(e.into()),
    }
}

/// Stamp `erasures` on the merged database and clear its restore state, in
/// one transaction; returns the state it cleared.
fn verified(conn: &mut Connection, erasures: u64) -> Result<Option<String>> {
    let tx = connection::write_transaction(conn)?;
    let cleared = restore_state::read(&tx)?;
    identity::stamp(&tx, erasures)?;
    restore_state::clear(&tx)?;
    tx.commit()?;
    Ok(cleared)
}
