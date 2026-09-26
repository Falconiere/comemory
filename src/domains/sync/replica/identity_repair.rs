//! The repair [`super::ensure`] runs on a database that is not the stream
//! `identity.json` names: the manifest's digests become erased barriers
//! again (digest-level — the full per-entity erase is `maintenance`'s, which
//! a supported restore runs), the database takes a new epoch and the
//! identity's device id, then the identity records the epoch. An
//! unestablished manifest also marks the database `erasure_unknown` in the
//! same transaction, so sync stays refused until `backup merge-erasures`.

use super::erasure_manifest;
use super::{
    Ensured, Identity, IdentityGuard, MANIFEST_FILE, REPLACED, count, file, now, rotate, stamp,
};
use crate::config::Paths;
use crate::domains::memories::save_lock::SaveGuard;
use crate::domains::sync::replica::restore_state::{self, State};
use crate::prelude::*;
use crate::store::{Connection, erase_rows, random_id, replica_journal};

/// Re-epoch the database behind `conn` and merge the manifest into it. The
/// guards are the proof both locks are held, in the lock order.
///
/// # Errors
/// SQLite, filesystem and JSON failures; nothing is recorded in
/// `identity.json` unless the database committed.
pub(super) fn run(
    _save: &SaveGuard,
    held: &IdentityGuard,
    paths: &Paths,
    conn: &mut Connection,
    mut identity: Identity,
) -> Result<Ensured> {
    let manifest = erasure_manifest::read(&file(paths, MANIFEST_FILE))?;
    let established = manifest
        .as_ref()
        .is_some_and(|m| m.established(identity.erasures));
    let lines = manifest.map(|m| m.lines).unwrap_or_default();
    let erasures = identity.erasures.max(count(lines.len()));
    let epoch = random_id::random_hex(16)?;
    let at = now()?;
    erase_rows::with_secure_delete(conn, |conn| {
        let tx = crate::store::connection::write_transaction(conn)?;
        erasure_manifest::bar_all(&tx, &lines, &at)?;
        replica_journal::set_identity(&tx, &epoch, &identity.device_id, &at)?;
        stamp(&tx, erasures)?;
        if established {
            restore_state::clear(&tx)?;
        } else {
            restore_state::set(&tx, State::ErasureUnknown)?;
        }
        tx.commit()?;
        Ok(())
    })?;
    rotate(
        held,
        paths,
        &mut identity,
        (&epoch, REPLACED, &at),
        erasures,
    )?;
    let merged = lines.len();
    if established {
        tracing::warn!(%epoch, merged, "replaced database: new stream epoch, erasures merged");
        Ok(Ensured::Reepoched { epoch, merged })
    } else {
        tracing::warn!(%epoch, merged, "replaced database: new stream epoch; the erasure manifest is not established, so what else was erased is unknown and sync is refused until `comemory backup merge-erasures`");
        Ok(Ensured::ErasureUnknown { epoch, merged })
    }
}
