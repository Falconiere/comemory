//! BM25 query wrapper around `index::Fts`. Exposes a single async-friendly
//! function the retrieval pipeline can call without taking on a SQLite
//! connection in its own state.

use std::path::Path;

use crate::index::Fts;
use crate::prelude::*;

/// Open the FTS5 database at `db_path` and return up to `limit` memory ids
/// ranked by BM25 relevance to `query`. When the database file does not yet
/// exist (no `comemory save` has run, or the file was deleted) we return an
/// empty list rather than erroring so the fused retrieval path can degrade to
/// vector-only without special-casing.
pub fn search_fts_ids(db_path: impl AsRef<Path>, query: &str, limit: usize) -> Result<Vec<String>> {
    if !db_path.as_ref().exists() {
        return Ok(Vec::new());
    }
    let fts = Fts::open(db_path)?;
    let hits = fts.search(query, limit)?;
    Ok(hits.into_iter().map(|h| h.id).collect())
}
