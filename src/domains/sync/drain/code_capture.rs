//! Capturing a code generation for upload, at push time (#252's design).
//!
//! For every repository the push may offer — approved canonical identity, a
//! real checkout, not skipped, `[sync] code_index` on — with no generation
//! already in flight: plan it on top of whatever generation the repository is
//! at under its canonical name (local or pulled), and when that differs,
//! record it `staged`, journal it and queue it in one transaction. It becomes
//! active here only once the upstream accepts it; a stale plan is left staged
//! and the next pass replans on top of what the pull activated.

use time::OffsetDateTime;

use crate::config::Config;
use crate::domains::code::generation;
use crate::domains::code::replica_payload::{
    CODE_ENTITY_KIND, CODE_PAYLOAD_VERSION, CodeGenerationV1,
};
use crate::domains::sync::code;
use crate::domains::sync::repository_policy::RepositoryPolicy;
use crate::prelude::*;
use crate::store::code_generation::{self, Generation, State};
use crate::store::replica_journal::{self, NewOperation, PayloadRef, ReplicaOp, ReplicaOrigin};
use crate::store::replica_outbox::{self, Scope};
use crate::store::{Connection, memory_row, replica_read, repo_marker};
use crate::utilities::operation_id;

/// Capture every repository whose index moved; returns how many were queued.
///
/// # Errors
/// Configuration and SQLite failures; a repository that cannot be planned is
/// skipped with a warning, never a failed pass.
pub fn capture(conn: &mut Connection, cfg: &Config, policy: &RepositoryPolicy) -> Result<usize> {
    if !cfg.sync.code_index {
        return Ok(0);
    }
    let skip = cfg.sync.skip_matcher()?;
    let mut queued = 0;
    for label in repo_marker::all_repos(conn)? {
        if skip.is_skipped(&label) || code::not_a_repository(conn, &label)?.is_some() {
            continue;
        }
        let Some(canonical) = policy.code_repository(&label).map(str::to_string) else {
            continue;
        };
        let in_flight = replica_outbox::read(conn, Scope::Entity(CODE_ENTITY_KIND, &canonical), 1)?;
        if !in_flight.is_empty() {
            continue;
        }
        match capture_one(conn, &label, &canonical) {
            Ok(true) => queued += 1,
            Ok(false) => {}
            Err(e) => tracing::warn!(repo = %label, error = %e, "code generation not captured"),
        }
    }
    Ok(queued)
}

/// Plan `label`'s index as the next generation of `canonical`; queue it when
/// it differs from the one the repository is at.
fn capture_one(conn: &mut Connection, label: &str, canonical: &str) -> Result<bool> {
    let Some(planned) = generation::plan(conn, label)? else {
        return Ok(false);
    };
    let active = code_generation::active(conn, canonical)?;
    let parent = active.as_ref().map(|g| g.generation_id.clone());
    let base = CodeGenerationV1::new(
        "",
        parent.as_deref(),
        &planned.generation.head,
        planned.generation.mined_commit.as_deref(),
        &planned.projection,
    );
    if let Some(active) = &active
        && same_contents(conn, &active.manifest_digest, &base)?
    {
        return Ok(false);
    }
    let generation_id = base.mint_id()?;
    let (bytes, digest) = base.with_id(&generation_id).canonical()?;
    let row = Generation {
        repo: canonical.to_string(),
        generation_id,
        parent_id: parent,
        state: State::Staged,
        origin: ReplicaOrigin::Local,
        manifest_digest: digest.clone(),
        ..planned.generation
    };
    journal(conn, &row, (&bytes, &digest))?;
    Ok(true)
}

/// Whether the generation stored under `digest` holds exactly `planned`'s
/// contents. Ids are left out: a generation's id is minted over its parent,
/// so an unchanged index planned on top of it always gets a new one. A
/// generation whose payload is not stored here counts as different.
fn same_contents(conn: &Connection, digest: &str, planned: &CodeGenerationV1) -> Result<bool> {
    let Some(bytes) = replica_read::payload_bytes(conn, digest)? else {
        return Ok(false);
    };
    let contents = |p: &CodeGenerationV1| {
        CodeGenerationV1 {
            generation_id: String::new(),
            parent_id: None,
            ..p.clone()
        }
        .canonical()
        .map(|(_, digest)| digest)
    };
    Ok(contents(&CodeGenerationV1::decode(&bytes)?)? == contents(planned)?)
}

/// Record a staged generation and journal and queue its upload, in one
/// transaction.
fn journal(conn: &mut Connection, row: &Generation, (bytes, digest): (&str, &str)) -> Result<()> {
    let at = memory_row::iso_format(OffsetDateTime::now_utc())?;
    let operation_id = operation_id::mint(CODE_ENTITY_KIND, &row.repo, ReplicaOp::Upsert.as_str());
    let new = NewOperation {
        operation_id: &operation_id,
        entity_kind: CODE_ENTITY_KIND,
        entity_key: &row.repo,
        op: ReplicaOp::Upsert,
        payload: Some(PayloadRef { digest, bytes }),
        schema_version: CODE_PAYLOAD_VERSION,
        repository: Some(&row.repo),
        origin: ReplicaOrigin::Local,
        at: &at,
    };
    let tx = conn.transaction()?;
    code_generation::record(&tx, row, &at)?;
    let epoch = replica_journal::stream_epoch(&tx)?;
    replica_journal::append(&tx, &epoch, &new)?;
    replica_outbox::enqueue(&tx, &new, None)?;
    tx.commit()?;
    Ok(())
}

/// Make an accepted generation this machine's active one for `canonical`.
///
/// # Errors
/// Propagates SQLite failures.
pub fn activate(conn: &Connection, canonical: &str, digest: &str, at: &str) -> Result<()> {
    let Some(generation) = code_generation::all(conn, canonical)?
        .into_iter()
        .find(|g| g.manifest_digest == digest)
    else {
        return Ok(());
    };
    code_generation::activate_following(conn, canonical, &generation.generation_id, at)
}

#[cfg(test)]
#[path = "tests/code_capture.rs"]
mod tests;
