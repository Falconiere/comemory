//! Journal seeding for documents indexed before the journal existed (or
//! before their repository was approved).
//!
//! Re-runs `domains::documents::journal`'s index-time capture for every
//! `documents` row with no `document_share` row, re-extracting the source
//! file so it journals the same revision the writer would have. A changed or
//! vanished file is left to the next `comemory index`; a still-withheld one
//! stays withheld. The scan restarts once the approval map's fingerprint
//! changes, so a later approval revisits what it had to withhold.

use std::path::Path;

use crate::domains::documents::document::extract;
use crate::domains::documents::journal::{self, IndexedRevision};
use crate::domains::documents::source::classify::{Classification, classify};
use crate::domains::sync::replica::bootstrap::{Progress, STATE_COMPLETE};
use crate::prelude::*;
use crate::store::connection::write_transaction;
use crate::store::{Connection, documents, repository_approval, schema_meta, seed_scan, sources};
use crate::utilities::context::Ctx;

/// Documents scanned per call — the same batch size [`super::bootstrap`]
/// uses.
const BATCH: usize = 200;

/// `schema_meta` key holding `pending`, `seeding` or `complete`.
pub const STATE_KEY: &str = "replica_seed_documents_state";

/// `schema_meta` key holding the last scanned `documents.id`.
pub const THROUGH_KEY: &str = "replica_seed_documents_through";

/// `schema_meta` key holding the approval fingerprint the scan last
/// completed under.
pub const POLICY_KEY: &str = "replica_seed_documents_policy";

/// Read the stored progress, without resolving whether it is still current
/// against the live approval map — [`advance`] does that.
///
/// # Errors
/// Propagates SQLite failures.
pub fn progress(ctx: &mut Ctx<'_>) -> Result<Progress> {
    let conn = ctx.conn()?;
    let state = schema_meta::get(conn, STATE_KEY)?.unwrap_or_else(|| "pending".to_string());
    let through = schema_meta::get(conn, THROUGH_KEY)?.unwrap_or_default();
    Ok(Progress { state, through })
}

/// Seed up to [`BATCH`] unshared documents and return the new progress.
/// Restarts from the beginning when the approval map changed since the last
/// completed scan.
///
/// # Errors
/// Propagates markdown, extraction and SQLite failures.
pub fn advance(ctx: &mut Ctx<'_>) -> Result<Progress> {
    let mut current = progress(ctx)?;
    let fingerprint = {
        let conn = ctx.conn()?;
        repository_approval::fingerprint(conn)?
    };
    if current.complete() {
        let recorded = {
            let conn = ctx.conn()?;
            schema_meta::get(conn, POLICY_KEY)?
        };
        if recorded.as_deref() == Some(fingerprint.as_str()) {
            return Ok(current);
        }
        // The approval map moved on: rescan from the start under the new
        // fingerprint, so a document approval unblocks is seeded once.
        current = Progress {
            state: "pending".to_string(),
            through: String::new(),
        };
    }
    let batch = {
        let conn = ctx.conn()?;
        seed_scan::unshared_documents_after(conn, &current.through, BATCH)?
    };
    let Some(last) = batch.last().map(|d| d.document_id.clone()) else {
        return mark(ctx, STATE_COMPLETE, &current.through, &fingerprint);
    };
    let finished = batch.len() < BATCH;
    for candidate in &batch {
        seed_one(ctx, candidate)?;
    }
    mark(
        ctx,
        if finished { STATE_COMPLETE } else { "seeding" },
        &last,
        &fingerprint,
    )
}

/// Re-extract and journal one candidate document, unless it already has a
/// revision (settled by a concurrent index run) or its file no longer
/// matches the indexed hash.
fn seed_one(ctx: &mut Ctx<'_>, candidate: &seed_scan::UnsharedDocument) -> Result<()> {
    let resolved = {
        let conn = ctx.conn()?;
        resolve_source(conn, candidate)?
    };
    let Some((label, source_root, relative_path)) = resolved else {
        return Ok(());
    };
    let path = source_root.join(&relative_path);
    let Ok(bytes) = std::fs::read(&path) else {
        return Ok(());
    };
    let hash = crate::utilities::digest::sha256_hex(&bytes);
    if hash != candidate.revision_hash {
        return Ok(());
    }
    let Classification::Document(format) = classify(&path, &bytes) else {
        return Ok(());
    };
    let file_stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(&candidate.document_id);
    let extracted = extract::extract(format, &bytes, file_stem)?;
    let at = crate::store::memory_row::iso_format(time::OffsetDateTime::now_utc())?;
    let conn = ctx.conn()?;
    let tx = write_transaction(conn)?;
    journal::record_revision(
        &tx,
        &IndexedRevision {
            label: label.as_deref(),
            source_root: &source_root,
            relative_path: &relative_path,
            document_id: &candidate.document_id,
            revision_hash: &candidate.revision_hash,
            extracted: &extracted,
            at: &at,
        },
    )?;
    tx.commit()?;
    Ok(())
}

/// The label, absolute source root and path-relative-to-root a candidate's
/// `source_files` row resolves to, or `None` when the source or its root is
/// gone (a source unregistered since indexing).
fn resolve_source(
    conn: &Connection,
    candidate: &seed_scan::UnsharedDocument,
) -> Result<Option<(Option<String>, std::path::PathBuf, String)>> {
    if documents::get_document(conn, &candidate.document_id)?.is_none() {
        return Ok(None);
    }
    let Some(file) = sources::get_file(conn, &candidate.source_file_id)? else {
        return Ok(None);
    };
    let Some(root) = sources::get(conn, &file.source_id)? else {
        return Ok(None);
    };
    Ok(Some((
        root.repo,
        Path::new(&root.canonical_path).to_path_buf(),
        file.relative_path,
    )))
}

/// Store the progress markers.
fn mark(ctx: &mut Ctx<'_>, state: &str, through: &str, fingerprint: &str) -> Result<Progress> {
    let conn = ctx.conn()?;
    schema_meta::upsert(conn, STATE_KEY, state)?;
    schema_meta::upsert(conn, THROUGH_KEY, through)?;
    if state == STATE_COMPLETE {
        schema_meta::upsert(conn, POLICY_KEY, fingerprint)?;
    }
    Ok(Progress {
        state: state.to_string(),
        through: through.to_string(),
    })
}

#[cfg(test)]
#[path = "tests/seed_documents.rs"]
mod tests;
