//! Hybrid retrieval over the v0.2 SQLite + sqlite-vec store, plus the six
//! command cores built on it.
//!
//! [`router`][ro] picks between the pure-vector, pure-lexical, and hybrid
//! (RRF-fused) branches based on whether the caller supplied a vector
//! and/or a non-empty query; [`code_route`][cro] is its `code_symbols`-side
//! sibling (BM25 + thresholded ANN + RRF, no relaxation ladder) and
//! [`doc_route`][dro] the document one. [`fuse`][fu] is the Reciprocal Rank
//! Fusion helper used when a caller wants to merge two ranked id lists.
//! [`bundle`][bu] shapes the JSON emitted by `comemory context`.
//! [`rerank`][re] is the second pipeline stage: it multiplies the fused
//! relevance by bounded deterministic priors (activation, feedback,
//! quality, supersede) built from the [`score`][sc] primitives;
//! [`code_rerank`][cre] is its code-side sibling (the four
//! [`code_prior`][cpr] boosts — PageRank, activation, working-set affinity,
//! feedback — + chunk→parent coalescing; `bundle` reuses the same priors,
//! relevance-free, to rank context code refs). [`diversify`][di] is the
//! third stage: SimHash near-duplicate collapse followed by Jaccard-MMR
//! greedy selection up to top-k. [`pipeline`][pi] chains all three stages
//! (route → rerank → diversify → top-k) and bumps access tracking, and
//! [`unified`][un] fuses the three legs for `comemory find`.
//! [`learned_rerank`][lr] is the optional fourth stage: when `[rerank]` is
//! enabled it reorders a fixed leading prefix of the deterministic ranking
//! through an out-of-process scorer, reporting itself through
//! [`learned_report`][lrp] and pausing through [`staged`][st] so a shared
//! connection lock is never held across inference. The cores both
//! delivery adapters call are `search`, `search_code`, `context`, `find`,
//! `suggest` and the console-only `config_retrieval`; clap flags, TTY
//! colouring and HTTP status mapping stay in `cli` and `serve`.
//!
//! [bu]: crate::domains::retrieval::bundle
//! [cpr]: crate::domains::retrieval::code_prior
//! [cre]: crate::domains::retrieval::code_rerank
//! [cro]: crate::domains::retrieval::code_route
//! [di]: crate::domains::retrieval::diversify
//! [dro]: crate::domains::retrieval::doc_route
//! [fu]: crate::domains::retrieval::fuse
//! [lr]: crate::domains::retrieval::learned_rerank
//! [lrp]: crate::domains::retrieval::learned_report
//! [st]: crate::domains::retrieval::staged
//! [pi]: crate::domains::retrieval::pipeline
//! [re]: crate::domains::retrieval::rerank
//! [ro]: crate::domains::retrieval::router
//! [sc]: crate::domains::retrieval::score
//! [un]: crate::domains::retrieval::unified

pub mod bundle;
pub mod code_prior;
/// Collect a memory's walked reference edges (+ pinned anchors) into ranked-ready refs.
pub mod code_ref_collect;
/// Per-repo current-state lookups (root, HEAD blob, index currency) for code-ref freshness.
pub mod code_ref_fetch;
/// Freshness (`fresh|stale|ghost|unpinned|unknown`) classification of pinned code refs.
pub mod code_ref_status;
pub mod code_rerank;
pub mod code_route;
pub mod code_search;
/// The owned value `comemory search-code` produces.
pub mod code_search_result;
/// `GET|PUT /config/retrieval`: the live ranking knobs.
pub mod config_retrieval;
/// `comemory context`: headline memory + code bundle for a query.
pub mod context;
/// The owned value `comemory context` produces.
pub mod context_result;
pub mod diversify;
/// The document retrieval leg: BM25 over `document_fts`, chunk hits
/// coalesced to their parent document.
pub mod doc_route;
/// The explain strip derived from a hit's `score_parts`: the log-magnitude
/// share partition over the multiplicative priors.
pub mod explain;
/// `comemory find`: one ranked list across memory, code, and documents.
pub mod find;
pub mod fuse;
pub mod graph_route;
/// What a learned ordering stage did to one requested search.
pub mod learned_report;
/// The one optional learned ordering stage every search surface shares.
pub mod learned_rerank;
pub mod pipeline;
pub mod rerank;
pub mod router;
/// Created-date window (`--since` / `--until` / `--as-of`) and the
/// `--only` domain scope, shared by every leg.
pub mod scope;
pub mod score;
/// `comemory search`: hybrid memory retrieval.
pub mod search;
/// `comemory search-code`: ranked code search.
pub mod search_code;
/// The owned value `comemory search` produces.
pub mod search_result;
/// The pause point between a deterministic ranking and its learned order.
pub mod staged;
/// `GET /search/suggest`: mined expansions + recent queries.
pub mod suggest;
/// Unified retrieval across memory, code, and documents (`comemory find`).
pub mod unified;
