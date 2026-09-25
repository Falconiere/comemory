//! What a document mutation owes the `replica-v1` feed, written inside the
//! caller's own transaction.
//!
//! Capture happens HERE, at index time, and not at push time the way a code
//! generation's does. A deletion is the reason: once `tombstone` has run, the
//! local rows describing what was removed are gone, so a push that came later
//! would have nothing left to tell a peer about. Journalling inside the
//! transaction that makes the change also means the two cannot disagree — a
//! failure aborts the mutation rather than leaving a document that exists
//! locally but owes no upload.
//!
//! The revision is also queued on `replica_outbox` in the same transaction,
//! and the exchange client (#255) pushes it; the feed position stays the
//! durable record of what this machine made.
//!
//! Withholding is not an error. A document whose repository is unapproved,
//! unlabelled or unrooted is indexed and searchable exactly as before and
//! simply never leaves the machine, so every refusal below returns
//! `Ok(None)` rather than aborting the index.

use std::path::Path;

use crate::domains::documents::document::ExtractedDocument;
use crate::domains::documents::replica_payload::{
    ChunkWire, DOCUMENT_ENTITY_KIND, DOCUMENT_PAYLOAD_VERSION, DocumentRevisionV1, LinkWire,
};
use crate::domains::documents::share::{self, SharedName};
use crate::domains::documents::source::classify;
use crate::prelude::*;
use crate::store::Connection;
use crate::store::document_share::{self, Share};
use crate::store::replica_journal::{
    self, NewOperation, PayloadRef, ReplicaOp, ReplicaOrigin, stream_epoch,
};
use crate::store::{repo_marker, repository_approval};

/// One indexed document's offer to the feed — the writer's transaction hands
/// this over without the capture having to know anything else about it.
pub(crate) struct IndexedRevision<'a> {
    /// The source's operator-typed `--repo` label, if it has one.
    pub label: Option<&'a str>,
    /// The registered source root, already resolved by the caller.
    pub source_root: &'a Path,
    /// The document's path relative to that source root.
    pub relative_path: &'a str,
    /// The local `documents.id`.
    pub document_id: &'a str,
    /// The `documents.revision_hash` this revision carries.
    pub revision_hash: &'a str,
    /// What the extractor produced.
    pub extracted: &'a ExtractedDocument,
    /// Provenance time, the same value the `documents` row stores.
    pub at: &'a str,
}

/// Journal one revision of one local document, and record what it is called
/// upstream.
///
/// Returns the feed sequence, or `None` when the document is withheld — see
/// [`shared_name`] for every reason that can happen.
///
/// `at` is the mutation's provenance time, the same value the `documents` row
/// carries, so the row and the feed describe one event.
///
/// # Errors
/// Propagates SQLite failures and payload serialization. A path this machine
/// cannot name portably is NOT an error here: the document stays local.
pub(crate) fn record_revision(tx: &Connection, new: &IndexedRevision<'_>) -> Result<Option<i64>> {
    let Some(name) = shared_name(tx, new.label, new.source_root, new.relative_path)? else {
        return Ok(None);
    };
    let revision = revision_of(&name, new);
    if !claim_name(tx, new.document_id, &name, &revision, new.at) {
        return Ok(None);
    }
    let (bytes, digest) = revision.canonical()?;
    let sequence = journal(
        tx,
        &NewOperation {
            operation_id: &operation_id(&name.shared_id, ReplicaOp::Upsert),
            entity_kind: DOCUMENT_ENTITY_KIND,
            entity_key: &name.shared_id,
            op: ReplicaOp::Upsert,
            payload: Some(PayloadRef {
                digest: &digest,
                bytes: &bytes,
            }),
            schema_version: DOCUMENT_PAYLOAD_VERSION,
            repository: Some(&name.repo),
            origin: ReplicaOrigin::Local,
            at: new.at,
        },
    )?;
    Ok(Some(sequence))
}

/// Record what this document is called upstream. `false` when it stays local.
///
/// A name another local document already holds is refused, not merged
/// (`document_share::record`). The refusal leaves both documents indexed and
/// neither shared, which is the only safe answer: sharing one of them under a
/// name the other also computes would overwrite it on every peer.
fn claim_name(
    tx: &Connection,
    document_id: &str,
    name: &SharedName,
    revision: &DocumentRevisionV1,
    at: &str,
) -> bool {
    let blocked_reason = share::blocked_reason(revision);
    let share = Share {
        document_id: document_id.to_string(),
        repo: name.repo.clone(),
        shared_id: name.shared_id.clone(),
        path: name.path.clone(),
        blocked_reason,
    };
    if let Err(error) = document_share::record(tx, &share, at) {
        tracing::warn!(document_id, path = %name.path, %error, "document not shared");
        return false;
    }
    if let Some(rule) = &share.blocked_reason {
        tracing::warn!(document_id, path = %name.path, rule, "document withheld by the secret scan");
        return false;
    }
    true
}

/// Journal the removal of a document this machine had shared.
///
/// Read from the `document_share` row rather than resolving the label again:
/// that row is the record of what this document was actually called upstream,
/// so a tombstone names exactly what a revision named. It must therefore be
/// called BEFORE the `documents` row is deleted — the share row cascades away
/// with its parent.
///
/// Returns `None` when the document was never shared, or was withheld: a peer
/// that never received the revision must not be told to delete it.
///
/// # Errors
/// Propagates SQLite failures.
pub(crate) fn record_tombstone(
    tx: &Connection,
    document_id: &str,
    at: &str,
) -> Result<Option<i64>> {
    let Some(share) = document_share::by_document(tx, document_id)? else {
        return Ok(None);
    };
    if share.blocked_reason.is_some() {
        return Ok(None);
    }
    let sequence = journal(
        tx,
        &NewOperation {
            operation_id: &operation_id(&share.shared_id, ReplicaOp::Tombstone),
            entity_kind: DOCUMENT_ENTITY_KIND,
            entity_key: &share.shared_id,
            op: ReplicaOp::Tombstone,
            // A tombstone carries no payload: the bytes it would name are
            // exactly what the deletion removed.
            payload: None,
            schema_version: DOCUMENT_PAYLOAD_VERSION,
            repository: Some(&share.repo),
            origin: ReplicaOrigin::Local,
            at,
        },
    )?;
    Ok(Some(sequence))
}

/// The portable name this document may be shared under, or `None` with a
/// logged reason.
///
/// Four ways to be withheld, all of them normal: the source carries no label;
/// no policy has resolved that label to an approved repository; the repository
/// has no indexed root on this machine, so there is nothing to make the path
/// relative TO; or the path cannot be named portably at all (it escapes the
/// root, or is absolute).
fn shared_name(
    conn: &Connection,
    label: Option<&str>,
    source_root: &Path,
    relative_path: &str,
) -> Result<Option<SharedName>> {
    let Some(label) = label.map(str::trim).filter(|l| !l.is_empty()) else {
        return Ok(None);
    };
    let Some(canonical) = repository_approval::canonical_for(conn, label)? else {
        tracing::debug!(label, "document repository is not approved for sharing");
        return Ok(None);
    };
    let Some(root) = repo_marker::root_path(conn, label)? else {
        tracing::debug!(
            label,
            "document repository has no indexed root on this machine"
        );
        return Ok(None);
    };
    match share::name_for(&canonical, Path::new(&root), source_root, relative_path) {
        Ok(name) => Ok(Some(name)),
        Err(error) => {
            tracing::warn!(relative_path, %error, "document cannot be named portably");
            Ok(None)
        }
    }
}

/// Assemble the wire revision from what the extractor produced.
fn revision_of(name: &SharedName, new: &IndexedRevision<'_>) -> DocumentRevisionV1 {
    let extracted = new.extracted;
    let chunks: Vec<ChunkWire> = extracted
        .chunks
        .iter()
        .map(|c| ChunkWire {
            ordinal: i64::try_from(c.ordinal).unwrap_or(i64::MAX),
            heading_path: c.heading_path.join(" > "),
            char_start: i64::try_from(c.char_range.0).unwrap_or(i64::MAX),
            char_end: i64::try_from(c.char_range.1).unwrap_or(i64::MAX),
            line_start: i64::try_from(c.line_range.0).unwrap_or(i64::MAX),
            line_end: i64::try_from(c.line_range.1).unwrap_or(i64::MAX),
            simhash: c.simhash as i64,
            text: c.text.clone(),
        })
        .collect();
    DocumentRevisionV1 {
        shared_id: name.shared_id.clone(),
        repo: name.repo.clone(),
        path: name.path.clone(),
        title: extracted.title.clone(),
        format: format_name(new.relative_path, extracted),
        revision_hash: new.revision_hash.to_string(),
        links: links_of(extracted, &chunks),
        chunks,
    }
}

/// The wire `format` value: derived from the path by the same function that
/// decided the document was extractable, and falling back to what the
/// extractor reported when the extension is not one of the four.
fn format_name(relative_path: &str, extracted: &ExtractedDocument) -> String {
    classify::format_of_extension(Path::new(relative_path))
        .unwrap_or(extracted.format)
        .as_str()
        .to_string()
}

/// Attribute each extracted link to the chunk it was written in.
///
/// The extractor returns a flat, deduplicated target list, but the wire needs
/// the passage each link belongs to so a reader can cite it. Markdown chunk
/// text is the raw source, so the inline form is still there to find; a target
/// no chunk contains is dropped rather than attributed to a passage that does
/// not mention it.
fn links_of(extracted: &ExtractedDocument, chunks: &[ChunkWire]) -> Vec<LinkWire> {
    extracted
        .links
        .iter()
        .filter_map(|target| {
            let needle = format!("]({target})");
            let chunk = chunks.iter().find(|c| c.text.contains(&needle))?;
            Some(LinkWire {
                ordinal: chunk.ordinal,
                target: target.clone(),
            })
        })
        .collect()
}

/// Append the feed row for one mutation and queue the upload it owes, in the
/// caller's transaction — a revision journalled here but never queued would be
/// shared by nobody.
fn journal(tx: &Connection, new: &NewOperation<'_>) -> Result<i64> {
    let epoch = stream_epoch(tx)?;
    let sequence = replica_journal::append(tx, &epoch, new)?;
    crate::store::replica_outbox::enqueue(tx, new, None)?;
    Ok(sequence)
}

/// Mint the operation id for one document mutation.
fn operation_id(shared_id: &str, op: ReplicaOp) -> String {
    crate::utilities::operation_id::mint(DOCUMENT_ENTITY_KIND, shared_id, op.as_str())
}
