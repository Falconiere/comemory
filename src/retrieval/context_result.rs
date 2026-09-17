//! The owned value `comemory context` produces.
//!
//! `api::context::run` returns this so the CLI writers and the
//! `/api/v1/context` handler each build their own envelope from one value.
//! Retrieval's result model, not a rendering concern (#166).

use crate::retrieval::bundle::Bundle;
use crate::retrieval::scope::TimeScope;
use crate::utilities::pagination::PageMeta;

/// Everything `comemory context`'s render layer needs: `api::context::run`
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
