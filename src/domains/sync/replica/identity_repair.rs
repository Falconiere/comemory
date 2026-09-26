//! The repair [`super::ensure`] runs on a database that is not the stream
//! `identity.json` names: every digest the manifest lists becomes an erased
//! barrier again, the database takes a new epoch and the identity's device
//! id, and the identity records the epoch — in that order, so a crash
//! between the two leaves a mismatch the next read repairs again.
//!
//! The merge is digest-level: the payload rows, replay scratch and staged
//! sets. The full per-entity erase (`maintenance::erase::apply_locked`, which
//! a supported restore runs) is not reachable from this domain.

use super::erasure_manifest;
use super::{
    ERASURES_KEY, Ensured, Epoch, Identity, IdentityGuard, MANIFEST_FILE, REPLACED, count, file,
    now, write,
};
use crate::config::Paths;
use crate::domains::memories::save_lock::SaveGuard;
use crate::prelude::*;
use crate::store::{
    Connection, erase_rows, random_id, replica_journal, replica_redaction,
    replica_redaction_copies, schema_meta,
};

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
        let mut digests = Vec::new();
        for line in &lines {
            replica_redaction::bar(&tx, &line.digests, &line.kind, &at)?;
            digests.extend(line.digests.iter().cloned());
        }
        replica_redaction_copies::clear_replay_of(&tx, &digests)?;
        erase_rows::erase_staged_of(&tx, &digests)?;
        replica_journal::set_identity(&tx, &epoch, &identity.device_id, &at)?;
        schema_meta::upsert(&tx, ERASURES_KEY, &erasures.to_string())?;
        tx.commit()?;
        Ok(())
    })?;
    identity.epoch.clone_from(&epoch);
    identity.epochs.push(Epoch {
        epoch: epoch.clone(),
        since: at,
        reason: REPLACED.to_string(),
    });
    identity.erasures = erasures;
    write(held, paths, &identity)?;
    let merged = lines.len();
    if established {
        tracing::warn!(%epoch, merged, "replaced database: new stream epoch, erasures merged");
        Ok(Ensured::Reepoched { epoch, merged })
    } else {
        tracing::warn!(%epoch, merged, "replaced database: new stream epoch; the erasure manifest is not established, so what else was erased is unknown");
        Ok(Ensured::ErasureUnknown { epoch, merged })
    }
}
