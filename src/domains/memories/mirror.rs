//! The one path that mirrors a markdown memory record into `comemory.db`.
//!
//! Every writer — `save`, `update`'s in-place re-mirror (and through it
//! restore, `refresh_refs` and the sync frontmatter patch), `rebuild` and the
//! sync import — goes through
//! [`crate::domains::memories::mirror::insert_row`], so none of them can
//! silently lose the derived edges a memory owes. Derivation happens here, in
//! the domain: `store::memory_row` receives the result as row data and never
//! calls back up.

use crate::domains::graph::{cross_link, doc_link};
use crate::domains::memories::Frontmatter;
use crate::prelude::*;
use crate::store::{Connection, MemoryLinks, memory_row};

/// Derive the graph links `body` owes, then write the whole mirror row set
/// for one memory inside the caller's transaction.
///
/// Both derivation steps run BEFORE the first write, and neither can see a
/// different answer for having moved: the regex harvest touches no database at
/// all, and the document resolution reads only `documents` / `source_files`,
/// which this write never writes to. A derivation failure therefore leaves the
/// transaction untouched, and the statement order inside
/// `store::memory_row::insert` is exactly what it was when the store derived
/// these itself.
///
/// `conn` is the caller's [`crate::store::Transaction`] in every production
/// path; no connection or transaction is opened here.
pub fn insert_row(
    conn: &Connection,
    fm: &Frontmatter,
    body: &str,
    slug: &str,
    md_path: &str,
    tags: &[String],
) -> Result<()> {
    let refs = cross_link::extract_refs(body);
    let documents = doc_link::resolve_memory_documents(conn, &refs.files)?;
    let links = MemoryLinks {
        files: &refs.files,
        symbols: &refs.symbols,
        documents: &documents,
    };
    memory_row::insert(conn, fm, body, slug, md_path, tags, &links)
}

#[cfg(test)]
#[path = "tests/mirror.rs"]
mod tests;
