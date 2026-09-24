//! The persisted vocabularies the retrieval log and the feedback tables share.
//!
//! Every literal here is a stored string in `comemory.db`, so writers and
//! readers must name the same const instead of inlining the text. Shared on
//! purpose (#166): retrieval writes `retrieval_log.source`, the feedback
//! surfaces write `feedback_events.target_kind` / `provenance`, graph
//! reinforcement writes the sentinel query ids, and `eval` reads all three —
//! none of them may own the vocabulary.

/// The `retrieval_log.source` vocabulary: which command originated a
/// logged query. Writers pass these consts; readers (`eval::golden`,
/// `eval::mine`) bind them as SQL parameters instead of inlining the
/// literals.
pub(crate) mod source {
    /// `comemory search` (memory search).
    pub(crate) const SEARCH: &str = "search";
    /// `comemory context` (context bundles).
    pub(crate) const CONTEXT: &str = "context";
    /// `comemory search-code` (code search). Excluded from reformulation
    /// mining and golden-set harvesting — these rows can only earn
    /// code-target feedback.
    pub(crate) const SEARCH_CODE: &str = "search-code";
    /// `comemory find` (unified memory + code + document search). Unlike
    /// [`SEARCH_CODE`] this IS mined and harvested: a `find` run can earn
    /// memory-target feedback, so its reformulations carry the same signal
    /// a `search` run's do.
    pub(crate) const FIND: &str = "find";
}

/// The `feedback_events.target_kind` vocabulary: what kind of id the
/// (memory-era-named) `memory_id` column carries for one verdict row.
pub(crate) mod target {
    /// Memory id (8-hex).
    pub(crate) const MEMORY: &str = "memory";
    /// Text-encoded `code_symbols` rowid (see `crate::domains::learning::code_feedback`).
    pub(crate) const CODE: &str = "code";
}

/// `provenance` of a human-stated verdict; the column's own `DEFAULT`
/// (`0008_v8_reinforcement.sql`). Written by `comemory feedback`, and by
/// the two HTTP feedback routes when the wire field `source` is omitted or
/// `"explicit"`. Every writer names it explicitly since #130, so no INSERT
/// leans on the default.
pub(crate) const PROV_MANUAL: &str = "manual";

/// `provenance` of an observed verdict: an HTTP caller's
/// `source: "implicit"`, such as an answer citing the memory. Counted in
/// `learning/summary`'s `implicit_share` like the `auto_*` tags, and like
/// them never harvested into the golden set nor used to mark a query
/// succeeded for reformulation mining (`store::feedback`'s readers take
/// [`PROV_MANUAL`]).
pub(crate) const PROV_IMPLICIT: &str = "implicit";

/// `provenance` tag for implicit `used` feedback minted by the
/// co-activation reward (commits touching a memory's referenced files).
/// Distinguishes auto-reinforcement rows from the [`PROV_MANUAL`] rows
/// written by `comemory feedback`. Matches the column added in
/// `0008_v8_reinforcement.sql`.
pub(crate) const PROV_AUTO_COACTIVATION: &str = "auto_coactivation";

/// `provenance` for search→edit credit: memory appeared in a recent
/// `retrieval_log` page *and* a referenced file was touched in the mined
/// commits. Still excluded from golden harvest via the sentinel query id.
pub(crate) const PROV_AUTO_SEARCH_EDIT: &str = "auto_search_edit";

/// Sentinel `query_id` stamped on co-activation `feedback_events` rows.
/// Deliberately NOT a real `q-<yyyymmdd>-<8hex>` id: `eval::golden::harvest`
/// INNER JOINs `feedback_events.query_id = retrieval_log.query_id`, and this
/// sentinel has no `retrieval_log` row, so an auto-reinforced memory can
/// never mint a golden pair — closing the confirmation loop.
pub(crate) const COACTIVATION_QUERY_ID: &str = "auto-coactivation";

/// Sentinel `query_id` for search→edit implicit `used` rows. Same golden
/// exclusion contract as [`COACTIVATION_QUERY_ID`].
pub(crate) const SEARCH_EDIT_QUERY_ID: &str = "auto-search-edit";

/// The `replica_feed.entity_kind` values of the two shared event kinds
/// (#254). Here rather than beside their payload types because `store` must
/// name them too — retention and purge redact these kinds' journal copies —
/// and `store` may not import a domain.
pub(crate) mod entity {
    /// One feedback verdict (`domains::learning::replica_payload`).
    pub(crate) const FEEDBACK_EVENT: &str = "feedback_event";
    /// One recorded command run (`domains::sync::replica::activity_payload`).
    pub(crate) const ACTIVITY_EVENT: &str = "activity_event";
}
