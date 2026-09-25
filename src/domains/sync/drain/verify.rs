//! `comemory sync --action verify` on a `replica-v1` key: per kind the
//! upstream reads, its 256 bucket digests against the same buckets over the
//! key's bound live digests — the upstream as this client last knew it, so a
//! pending local edit is not a mismatch.
//!
//! A differing kind is repaired by a compacting replay of that kind alone
//! (the main cursor never moves), then compared again. What still differs is
//! reported beside the key's held positions, not looped on: an upstream that
//! lost data does not get it back from here (#256).

use std::collections::BTreeMap;

use serde::Serialize;

use crate::config::{Config, Paths};
use crate::domains::sync::drain::report::End;
use crate::domains::sync::drain::session::{Legs, Mode, Session};
use crate::domains::sync::drain::{network, rebootstrap, replica_pass};
use crate::domains::sync::exchange::manifest::bucket_digests;
use crate::domains::sync::replica::contract_views::ManifestResponse;
use crate::prelude::*;
use crate::store::Connection;
use crate::store::replica_binding;
use crate::store::replica_cursor;
use crate::store::replica_pull_hold;
use crate::store::sync_exchange::{self, ExchangeKey};

/// One kind's comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KindVerify {
    /// The entity kind.
    pub kind: String,
    /// Buckets that still differ after any repair.
    pub differing_buckets: u32,
    /// Whether a repair ran and the kind matches afterwards.
    pub repaired: bool,
}

/// The replica verify report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReplicaVerify {
    /// One row per kind the upstream reads.
    pub kinds: Vec<KindVerify>,
    /// Pull positions held for the key's current stream.
    pub held_positions: u32,
}

/// What a comparison needs to reopen a pass for a repair.
pub struct Reopen<'a> {
    /// The data directory.
    pub paths: &'a Paths,
    /// The configuration.
    pub cfg: &'a Config,
    /// Opens a fresh session (a new manifest) for the key.
    pub open: &'a dyn Fn(&mut Connection) -> Result<Option<Box<Session>>>,
}

/// Compare every kind, repair the differing ones, compare again.
///
/// # Errors
/// Store failures, and a session that could not be reopened for a repair.
pub fn run(
    conn: &mut Connection,
    reopen: &Reopen<'_>,
    session: &mut Session,
) -> Result<ReplicaVerify> {
    // Bindings describe the upstream up to the cursor; drain to the head the
    // manifest was taken at (finishing any replay in progress) first, so a
    // position not yet read is not reported as a difference.
    let pass = replica_pass::run(
        reopen.paths,
        reopen.cfg,
        conn,
        session,
        (Mode::Manual, Legs::Both),
    )?;
    let before = compare(conn, &session.key, manifest(session)?)?;
    let mut repaired = Vec::new();
    for (kind, differing) in &before {
        // A pull that did not reach the head (stalled, offline) differs for
        // that reason alone; a repair replay would not change it.
        if *differing == 0 || pass.end != End::CaughtUp {
            continue;
        }
        repair(conn, reopen, kind)?;
        repaired.push(kind.clone());
    }
    let after = if repaired.is_empty() {
        before
    } else {
        let session = (reopen.open)(conn)?.ok_or_else(|| {
            Error::Unavailable("verify could not reopen the session to compare again".into())
        })?;
        compare(conn, &session.key, manifest(&session)?)?
    };
    let kinds = after
        .into_iter()
        .map(|(kind, differing_buckets)| KindVerify {
            repaired: repaired.contains(&kind) && differing_buckets == 0,
            kind,
            differing_buckets,
        })
        .collect();
    Ok(ReplicaVerify {
        kinds,
        held_positions: held_positions(conn, &session.key)?,
    })
}

/// Replay `kind` alone from 0 in a fresh session (a new manifest), without
/// moving the main cursor.
fn repair(conn: &mut Connection, reopen: &Reopen<'_>, kind: &str) -> Result<()> {
    let Some(mut session) = (reopen.open)(conn)? else {
        return Err(Error::Unavailable(format!(
            "verify could not reopen the session to repair {kind}"
        )));
    };
    let head = manifest(&session)?.head_sequence;
    rebootstrap::start(&mut session.row, &format!("repair:{kind}"), head);
    sync_exchange::save(conn, &session.row, &network::now()?)?;
    let legs = (Mode::Manual, Legs::Both);
    replica_pass::run(reopen.paths, reopen.cfg, conn, &mut session, legs)?;
    Ok(())
}

/// The session's manifest.
fn manifest(session: &Session) -> Result<&ManifestResponse> {
    session
        .manifest
        .as_ref()
        .ok_or_else(|| Error::Other("a replica verify needs the manifest".into()))
}

/// Differing buckets per kind the upstream reads.
fn compare(
    conn: &Connection,
    key: &ExchangeKey,
    manifest: &ManifestResponse,
) -> Result<Vec<(String, u32)>> {
    let mut local: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for binding in replica_binding::all(conn, key)? {
        if let Some(digest) = binding.synced_digest.filter(|_| !binding.synced_deleted) {
            local.entry(binding.entity_kind).or_default().push(digest);
        }
    }
    let kinds = manifest
        .capabilities
        .iter()
        .filter_map(|c| c.split_once('@').map(|(kind, _)| kind.to_string()));
    let mut rows = Vec::new();
    for kind in kinds {
        let ours = bucket_digests(local.remove(&kind).unwrap_or_default());
        let theirs = manifest
            .entity_kinds
            .iter()
            .find(|k| k.kind == kind)
            .map_or_else(|| bucket_digests(Vec::new()), |k| k.buckets.clone());
        let differing = ours.iter().zip(&theirs).filter(|(a, b)| a != b).count();
        rows.push((kind, u32::try_from(differing).unwrap_or(u32::MAX)));
    }
    Ok(rows)
}

/// Pull positions held for the key's current stream.
fn held_positions(conn: &Connection, key: &ExchangeKey) -> Result<u32> {
    let Some(cursor) = replica_cursor::load(conn, &key.api_url, &key.workspace_id)? else {
        return Ok(0);
    };
    let holds = replica_pull_hold::list(conn, key, &cursor.stream_epoch)?;
    Ok(u32::try_from(holds.len()).unwrap_or(u32::MAX))
}

#[cfg(test)]
#[path = "tests/verify.rs"]
mod tests;
