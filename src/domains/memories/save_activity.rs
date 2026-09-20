//! The `save` command's activity summary: the request fields it reports and
//! the JSON it builds from them.
//!
//! Its own file beside `save.rs` so that file stays under the 300-line
//! ceiling (Binding Rule 3); nothing else belongs here.

use crate::domains::memories::save::Response;
use crate::utilities::activity::bounded_text;

/// The request fields the activity summary reports, kept past the move of
/// `Request` into the save itself.
pub(crate) struct Asked {
    /// The title the caller supplied, when it supplied one.
    pub(crate) title: Option<String>,
    /// The memory kind asked for.
    pub(crate) kind: String,
    /// The repo label asked for; empty when unscoped.
    pub(crate) repo: String,
    /// How many tags the request carried.
    pub(crate) tags: usize,
    /// How many memories the save claimed to supersede.
    pub(crate) supersedes: usize,
}

/// The bounded `save` summary: which memory was written and what shape the
/// caller asked for. Never the body — the feed says what happened, and the
/// memory itself is one `comemory show` away.
pub(crate) fn activity_summary(asked: &Asked, res: &Response) -> serde_json::Value {
    serde_json::json!({
        "id": res.id,
        "title": asked.title.as_deref().map(bounded_text),
        "kind": asked.kind,
        "created": res.created,
        "tags": asked.tags,
        "supersedes": asked.supersedes,
        "duplicate_of": res.duplicate_of,
    })
}
