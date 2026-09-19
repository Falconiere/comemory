//! The owned value `comemory context` produces, and the flattened-bundle
//! envelope both delivery adapters serialize it into.
//!
//! `retrieval::context::run` returns [`ContextResult`][cr] so the CLI writers
//! and the `/api/v1/context` handler each build their own view from one
//! value — and both build it with the same [`envelope`][en]. Retrieval's
//! result model, not a rendering concern (#166, #171); `output::context`
//! keeps the emitters.
//!
//! [cr]: crate::domains::retrieval::context_result::ContextResult
//! [en]: crate::domains::retrieval::context_result::envelope

use serde::Serialize;

use crate::domains::retrieval::bundle::Bundle;
use crate::domains::retrieval::scope::{ScopeEcho, TimeScope};
use crate::utilities::pagination::PageMeta;

/// Everything `comemory context`'s render layer needs: `retrieval::context::run`
/// returns this so the CLI and the HTTP handler can each build their own
/// envelope from one owned value instead of four loose parameters.
pub struct ContextResult {
    /// The assembled context bundle.
    pub bundle: Bundle,
    /// Id of the retrieval_log row for this lookup.
    pub query_id: Option<String>,
    /// Memory-list pagination cursor.
    pub meta: PageMeta,
    /// The lookup's time-scoping flags.
    pub scope: TimeScope,
}

/// JSON envelope returned to `--json` callers. The bundle fields stay at the
/// top level (flattened) so existing consumers keep reading `query` /
/// `memories` / `code_refs` / `relations` unchanged; `query_id` is added
/// alongside them, mirroring the `comemory search` envelope. The
/// pagination cursor (`limit` / `offset` / `has_more` / `total`) describes
/// the windowed `memories` list — `total` is the in-window ranked memory
/// count, and the per-memory `code_refs` are left unpaginated (every
/// surfaced memory keeps its full ref set).
#[derive(Serialize)]
pub struct Envelope<'a> {
    /// The assembled context bundle, flattened into the envelope root.
    #[serde(flatten)]
    pub bundle: &'a Bundle,
    /// Id of the retrieval_log row for this lookup; absent when logging
    /// was off or failed. Feed it back via `comemory feedback <id>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_id: Option<&'a str>,
    /// Requested memory-list page size.
    pub limit: usize,
    /// Number of leading ranked memories skipped.
    pub offset: usize,
    /// Whether more in-window ranked memories exist beyond this page.
    pub has_more: bool,
    /// In-window ranked memory count the page was sliced from; `None` when
    /// not cheaply known.
    pub total: Option<usize>,
    /// The lookup's time-scoping flags, echoed at the envelope root in the
    /// same shape `comemory search` uses. Skipped field-by-field when
    /// unset, so an unscoped lookup is unchanged.
    #[serde(flatten)]
    pub scope: ScopeEcho<'a>,
}

/// Build the serializable envelope. Public so both delivery adapters and the
/// mirror tests can pin the JSON contract without going through stdout.
/// `scope` echoes the time-scoping flags for this lookup
/// ([`ScopeEcho::default`] for an unscoped one).
pub fn envelope<'a>(
    bundle: &'a Bundle,
    query_id: Option<&'a str>,
    page: PageMeta,
    scope: ScopeEcho<'a>,
) -> Envelope<'a> {
    Envelope {
        bundle,
        query_id,
        limit: page.limit,
        offset: page.offset,
        has_more: page.has_more,
        total: page.total,
        scope,
    }
}
