//! Build one generation from what a local `index-code` run left behind.
//!
//! Pure assembly over the existing readers — `indexed_files` for the manifest,
//! `code_sync` for the symbols and edges, `repo_marker` for the head and the
//! mining cursor. The id is the digest of the canonical payload, so a second
//! index run over an unchanged tree produces the same id and offers nothing.

use crate::domains::code::replica_payload::CodeGenerationV1;
use crate::prelude::*;
use crate::store::code_generation::{Generation, State};
use crate::store::remote_code::{Edge, File, Projection, Symbol};
use crate::store::replica_journal::ReplicaOrigin;
use crate::store::{Connection, code_generation, code_sync, indexed_files, repo_marker};

/// A generation and the projection it names — what a push sends and what an
/// acceptance writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    /// The generation row.
    pub generation: Generation,
    /// Its manifest, symbols and edges.
    pub projection: Projection,
}

/// Assemble the current local generation for `repo`.
///
/// Returns `None` when the repo has no `repo_marker` row — it was never
/// indexed here, so there is nothing to offer. A repo the sync policy
/// withholds never reaches this function; that decision stays in
/// `sync::code`, which is the one place it is made.
///
/// # Errors
/// Propagates SQLite failures and canonical-payload serialization.
pub fn plan(conn: &Connection, repo: &str) -> Result<Option<Planned>> {
    let Some(head) = repo_marker::last_head(conn, repo)? else {
        return Ok(None);
    };
    let mined_commit = repo_marker::last_mined_commit(conn, repo)?;
    let projection = project(conn, repo)?;
    let parent_id = code_generation::active_local(conn, repo)?.map(|g| g.generation_id);
    // One derivation, in the payload that also verifies it on the receiving
    // side: the id is the 32-hex prefix of the payload's digest taken with
    // the id field blank, and `manifest_digest` is the digest of the payload
    // as it will be sent. Deriving either here would let the two drift.
    let payload = CodeGenerationV1::new(
        "",
        parent_id.as_deref(),
        &head,
        mined_commit.as_deref(),
        &projection,
    );
    let generation_id = payload.mint_id()?;
    let manifest_digest = payload.with_id(&generation_id).canonical()?.1;
    let file_count = i64::try_from(projection.files.len()).unwrap_or(i64::MAX);
    Ok(Some(Planned {
        generation: Generation {
            repo: repo.to_string(),
            generation_id,
            parent_id,
            head,
            mined_commit,
            origin: ReplicaOrigin::Local,
            state: State::Staged,
            file_count,
            manifest_digest,
        },
        projection,
    }))
}

/// Read the whole projection: the manifest, every file's symbols, its resolved
/// imports, and the repo's mined co-change pairs.
fn project(conn: &Connection, repo: &str) -> Result<Projection> {
    let mut files: Vec<File> = indexed_files::list_for_repo(conn, repo)?
        .into_iter()
        .map(|(path, blob_oid)| File { path, blob_oid })
        .collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));

    let mut symbols = Vec::new();
    let mut edges = Vec::new();
    for file in &files {
        symbols.extend(symbols_of(conn, repo, file)?);
        edges.extend(imports_of(conn, repo, file)?);
    }
    edges.extend(co_changes_of(conn, repo)?);

    symbols.sort_by(|a, b| {
        (&a.path, a.line_start, &a.symbol).cmp(&(&b.path, b.line_start, &b.symbol))
    });
    edges.sort_by(|a, b| {
        (&a.rel, &a.src_path, &a.dst_path).cmp(&(&b.rel, &b.src_path, &b.dst_path))
    });
    Ok(Projection {
        files,
        symbols,
        edges,
    })
}

/// One file's top-level symbols, snippet-free.
fn symbols_of(conn: &Connection, repo: &str, file: &File) -> Result<Vec<Symbol>> {
    Ok(code_sync::parent_symbols_for_file(conn, repo, &file.path)?
        .into_iter()
        .map(|row| Symbol {
            path: file.path.clone(),
            symbol: row.symbol,
            kind: row.kind,
            lang: row.lang,
            line_start: row.line_start,
            line_end: row.line_end,
        })
        .collect())
}

/// One file's resolved imports, each anchored at the blob it was resolved
/// from so a reader can tell which revision the edge describes.
fn imports_of(conn: &Connection, repo: &str, file: &File) -> Result<Vec<Edge>> {
    Ok(code_sync::import_targets(conn, repo, &file.path)?
        .into_iter()
        .map(|dst_path| Edge {
            rel: "imports".to_string(),
            src_path: file.path.clone(),
            dst_path,
            weight: 1,
            anchor: Some(file.blob_oid.clone()),
        })
        .collect())
}

/// The repo's mined co-change pairs, anchored at the commit they were mined
/// through.
fn co_changes_of(conn: &Connection, repo: &str) -> Result<Vec<Edge>> {
    let mined = repo_marker::last_mined_commit(conn, repo)?;
    Ok(code_sync::co_changed_pairs(conn, repo)?
        .into_iter()
        .map(|(src_path, dst_path, weight)| Edge {
            rel: "co_changed".to_string(),
            src_path,
            dst_path,
            weight,
            anchor: mined.clone(),
        })
        .collect())
}

#[cfg(test)]
#[path = "tests/generation.rs"]
mod tests;
