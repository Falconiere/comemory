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
    // The chain belongs to the REPOSITORY, not to an origin: a generation
    // extends whatever the repo is currently at, whoever built it. Deriving
    // the parent from the locally built generation only would mute this
    // machine for good the first time a peer's generation became active —
    // its next plan would claim to be the repo's first, and no peer could
    // accept that. Which generation may be UPLOADED is the separate question
    // `drain::code_capture` answers: only one planned here from this
    // machine's own index, never a pulled one it already holds.
    let parent_id = code_generation::active(conn, repo)?.map(|g| g.generation_id);
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
///
/// Every list is sorted before it is returned: the generation's id is minted
/// from these bytes, so two runs over the same tree must agree on their order
/// as well as their contents.
fn project(conn: &Connection, repo: &str) -> Result<Projection> {
    let mut files: Vec<File> = indexed_files::list_for_repo(conn, repo)?
        .into_iter()
        .map(|(path, blob_oid)| File { path, blob_oid })
        .collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));

    let mut symbols = Vec::new();
    for file in &files {
        for row in code_sync::parent_symbols_for_file(conn, repo, &file.path)? {
            symbols.push(Symbol {
                path: file.path.clone(),
                symbol: row.symbol,
                kind: row.kind,
                lang: row.lang,
                line_start: row.line_start,
                line_end: row.line_end,
            });
        }
    }
    symbols.sort_by(|a, b| {
        (&a.path, a.line_start, &a.symbol).cmp(&(&b.path, b.line_start, &b.symbol))
    });

    let mut edges = edges_of(conn, repo, &files)?;
    edges.sort_by(|a, b| {
        (&a.rel, &a.src_path, &a.dst_path).cmp(&(&b.rel, &b.src_path, &b.dst_path))
    });
    Ok(Projection {
        files,
        symbols,
        edges,
    })
}

/// Every edge the local index can state, each carrying the revision it
/// describes: a resolved import is anchored at the blob it was resolved from,
/// and a mined co-change pair at the commit it was mined through.
fn edges_of(conn: &Connection, repo: &str, files: &[File]) -> Result<Vec<Edge>> {
    let mut edges = Vec::new();
    for file in files {
        for dst_path in code_sync::import_targets(conn, repo, &file.path)? {
            edges.push(Edge {
                rel: "imports".to_string(),
                src_path: file.path.clone(),
                dst_path,
                weight: 1,
                anchor: Some(file.blob_oid.clone()),
            });
        }
    }
    let mined = repo_marker::last_mined_commit(conn, repo)?;
    for (src_path, dst_path, weight) in code_sync::co_changed_pairs(conn, repo)? {
        edges.push(Edge {
            rel: "co_changed".to_string(),
            src_path,
            dst_path,
            weight,
            anchor: mined.clone(),
        });
    }
    Ok(edges)
}

#[cfg(test)]
#[path = "tests/generation.rs"]
mod tests;
