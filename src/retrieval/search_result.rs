//! The owned value `comemory search` produces.
//!
//! `api::search::run` returns this so the CLI's `--json`/TTY writers and the
//! `/api/v1/search` handler each build their own envelope from one value
//! instead of five loose parameters. It is the command's result model, not a
//! rendering concern, so it belongs to retrieval rather than to `output`
//! (#166).

use std::collections::HashMap;

use crate::retrieval::rerank::Reranked;
use crate::retrieval::scope::TimeScope;
use crate::store::memory_meta::MemoryMeta;
use crate::utilities::pagination::PageMeta;

/// Everything `comemory search`'s render layer needs: `api::search::run`
/// returns this so the CLI and the HTTP handler can each build their own
/// envelope (`--json` stdout vs the `/api/v1` response `data` field) from
/// one owned value instead of five loose parameters.
pub struct SearchResult {
    /// Reranked + diversified hits for the requested page.
    pub hits: Vec<Reranked>,
    /// Id of the retrieval_log row for this run.
    pub query_id: Option<String>,
    /// Pagination cursor for the returned page.
    pub meta: PageMeta,
    /// Batched navigation metadata for `hits`, keyed by memory id.
    pub nav: HashMap<String, MemoryMeta>,
    /// The run's time-scoping flags.
    pub scope: TimeScope,
}
