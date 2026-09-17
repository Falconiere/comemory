//! The owned value `comemory search-code` produces.
//!
//! `api::search_code::run` returns this so the CLI writers and the
//! `/api/v1/search/code` handler each build their own envelope from one value.
//! Retrieval's result model, not a rendering concern (#166).

use crate::retrieval::code_rerank::CodeReranked;
use crate::utilities::pagination::PageMeta;

/// Everything `comemory search-code`'s render layer needs: `api::search_code::run`
/// returns this so the CLI and the HTTP handler can each build their own
/// envelope (`--json` stdout vs the `/api/v1` response `data` field) from
/// one owned value instead of four loose parameters.
pub struct SearchCodeResult {
    /// Reranked hits for the requested page.
    pub hits: Vec<CodeReranked>,
    /// Id of the `retrieval_log` row for this run; `None` when tracking was
    /// off or logging failed.
    pub query_id: Option<String>,
    /// Pagination cursor for the returned page.
    pub meta: PageMeta,
    /// `true` when the page is empty because `code_symbols` has no rows at
    /// all (as opposed to a query that simply matched nothing) — the CLI
    /// TTY view uses this to print an `index-code` hint instead of silent
    /// emptiness.
    pub index_empty: bool,
}
