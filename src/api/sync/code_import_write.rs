//! Per-file writes behind `code_import::run`: replace one file's symbol
//! rows and outgoing `imports` edges, or remove a file entirely.
//!
//! Rows land through the same `code_row` writers `index-code` uses, with an
//! empty snippet and a zero SimHash — the projection carries no source, so
//! there is nothing to hash and nothing for `code_fts` to index. A workspace
//! filled this way answers the graph and the repo inventory; code search
//! stays where the source is.

use crate::api::sync::code_types::CodeFileWire;
use crate::prelude::*;
use crate::store::code_row::{self, CodeSymbolRow};
use crate::store::edges::{self, EdgeKey, file_node_id};
use crate::store::{Transaction, indexed_files};

/// Replace `file` under `repo`, or do nothing when the stored blob already
/// matches — two machines on the same commit converge instead of fighting.
pub(crate) fn write_file(tx: &Transaction<'_>, repo: &str, file: &CodeFileWire) -> Result<()> {
    if indexed_files::blob_oid_for(tx, repo, &file.path)?.as_deref() == Some(file.blob_oid.as_str())
    {
        return Ok(());
    }
    let src = file_node_id(repo, &file.path);
    code_row::purge_file_symbols(tx, repo, &file.path)?;
    edges::delete_imports_from(tx, &src)?;
    for s in &file.symbols {
        code_row::insert(
            tx,
            &CodeSymbolRow {
                repo,
                path: &file.path,
                blob_oid: &file.blob_oid,
                symbol: &s.symbol,
                kind: &s.kind,
                lang: &s.lang,
                line_start: s.line_start,
                line_end: s.line_end,
                snippet: "",
                simhash: 0,
                parent_id: None,
            },
        )?;
    }
    for target in &file.imports {
        if target == &file.path {
            continue;
        }
        let dst = file_node_id(repo, target);
        edges::insert(
            tx,
            EdgeKey {
                src_kind: "file",
                src_id: &src,
                dst_kind: "file",
                dst_id: &dst,
                rel: "imports",
            },
        )?;
    }
    code_row::upsert_indexed_file(tx, repo, &file.path, &file.blob_oid)
}

/// Drop every row and every file-node edge of `path`. Memory citations
/// (`references_file` / `references_symbol`, bare ids) are the memory's own
/// and survive, exactly as `store::repo_drop` keeps them.
///
/// The three deletes are independent — `edges`, `code_symbols` and
/// `indexed_files` reference one another by value, never by foreign key —
/// so their order is a reading order, not a dependency: edges first, since
/// they are what the graph draws, then the rows, then the cursor.
pub(crate) fn remove_file(tx: &Transaction<'_>, repo: &str, path: &str) -> Result<()> {
    edges::delete_touching(tx, "file", &file_node_id(repo, path))?;
    code_row::purge_file_symbols(tx, repo, path)?;
    indexed_files::delete_one(tx, repo, path)
}
