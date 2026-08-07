//! `api::unindex::{Request, run}` — the shared middle of `comemory unindex`
//! / `DELETE /api/v1/sources`: unregister a document source and delete its
//! derived rows. External files under the source root are never touched.
//! Moved out of `cli::unindex::run` (Binding Rule 1).

use serde::{Deserialize, Serialize};

use crate::api::Ctx;
use crate::graph::edges;
use crate::prelude::*;
use crate::source::registry::Registry;
use crate::store::{document_fts, documents, sources};

/// `comemory unindex` / `DELETE /api/v1/sources` request.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// The source id (see `comemory sources`) or the path it was registered
    /// under.
    pub target: String,
}

/// Report of one `unindex` run.
#[derive(Serialize, Debug)]
pub struct Response {
    /// The unregistered source's id.
    pub source_id: String,
    /// The canonical path it was registered under.
    pub canonical_path: String,
    /// Number of documents whose derived rows were removed.
    pub documents_removed: usize,
}

/// Unregister the source matching `req.target` and delete its derived rows
/// (`source_files`/`documents`/`document_chunks`/`document_fts`) plus the
/// `member_of_source`/`references_document` edges those rows own.
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Response> {
    let registry = Registry::new(ctx.paths.clone());
    let conn = ctx.conn()?;

    let Some(entry) = registry.unregister(&req.target)? else {
        return Err(Error::NotFound(format!(
            "no registered source matches `{}`",
            req.target
        )));
    };

    // document_fts is a virtual table with no FK, so its rows must be
    // deleted explicitly; `sources::delete` below cascades
    // source_files -> documents -> document_chunks via `ON DELETE CASCADE`.
    // `edges` carries no FK to either table, so both edge kinds the link
    // deriver wrote (`member_of_source` keyed by file id, `references_document`
    // keyed by document id) need the same explicit purge.
    let doc_ids = documents::document_ids_for_source(conn, entry.id.as_str())?;
    for doc_id in &doc_ids {
        document_fts::delete_document(conn, doc_id)?;
        edges::delete_touching(conn, "document", doc_id)?;
    }
    for file in sources::list_files_by_source(conn, entry.id.as_str())? {
        edges::delete_touching(conn, "file", &file.id)?;
    }
    sources::delete(conn, entry.id.as_str())?;

    Ok(Response {
        source_id: entry.id.to_string(),
        canonical_path: entry.canonical_path.to_string_lossy().into_owned(),
        documents_removed: doc_ids.len(),
    })
}
