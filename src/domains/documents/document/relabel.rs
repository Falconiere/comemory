//! Update an unchanged document's repository and its replication identity.

use std::path::Path;

use super::fingerprint::{self, FileStat};
use super::{DocumentFormat, extract};
use crate::domains::documents::journal::{self, IndexedRevision};
use crate::domains::graph::doc_link;
use crate::prelude::*;
use crate::store::{Connection, document_share, documents};

pub(super) fn apply(
    conn: &mut Connection,
    stat: &FileStat<'_>,
    root: &Path,
    repo: Option<&str>,
    format: DocumentFormat,
    bytes: &[u8],
    hash: &str,
) -> Result<()> {
    let id = fingerprint::document_id_of(stat.file_id);
    let Some(old) = documents::get_document(conn, &id)? else {
        return Ok(());
    };
    if old.repo.as_deref() == repo {
        return Ok(());
    }
    let stem = Path::new(stat.relative_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(stat.relative_path);
    let extracted = extract::extract(format, bytes, stem)?;
    let now = super::writer::iso_now()?;
    let tx = conn.transaction()?;
    journal::record_tombstone(&tx, &id, &now)?;
    document_share::remove(&tx, &id)?;
    documents::upsert_document(
        &tx,
        documents::DocumentUpsert {
            id: &id,
            source_file_id: stat.file_id,
            title: &extracted.title,
            repo,
            revision_hash: hash,
            created_at: &old.created_at,
            updated_at: &now,
        },
    )?;
    journal::record_revision(
        &tx,
        &IndexedRevision {
            label: repo,
            source_root: root,
            relative_path: stat.relative_path,
            document_id: &id,
            revision_hash: hash,
            extracted: &extracted,
            at: &now,
        },
    )?;
    tx.commit()?;
    doc_link::derive_after_document(
        conn,
        stat.source_id,
        stat.file_id,
        &id,
        repo,
        stat.relative_path,
        &extracted.links,
    )?;
    Ok(())
}
