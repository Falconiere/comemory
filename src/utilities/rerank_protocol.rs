//! The versioned reranker request/response wire types (#211).
//!
//! One child process per request: the request is one JSON object on the
//! child's stdin followed by EOF, the response one JSON object on its stdout
//! followed by EOF and a zero exit. stderr is diagnostic only and never
//! parsed. Both objects `deny_unknown_fields`, so an additive field is a
//! [`RERANK_PROTOCOL_VERSION`] bump rather than a silent extension.
//!
//! A candidate id is an opaque, domain-qualified string this module never
//! parses; what it means belongs to the candidate observation contract (#208).
//! The full field-by-field contract is
//! `docs/designs/2026-09-18-reranker-command-protocol.md` § Wire protocol.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::utilities::dated_id::{dated_id, is_valid_dated_id};

/// The protocol version this build speaks and requires back.
pub const RERANK_PROTOCOL_VERSION: u32 = 1;

/// What distinguishes a reranker request id from a retrieval query id.
const REQUEST_ID_PREFIX: &str = "rr";

/// One candidate offered for scoring, in the caller's deterministic order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RerankCandidate {
    /// Opaque, domain-qualified, unique within the request.
    pub id: String,
    /// Zero-based position in the caller's deterministic order, equal to this
    /// candidate's array index. Carried explicitly so the tie-break survives a
    /// scorer that reorders its echo.
    pub rank: u32,
    /// The bounded candidate text. The caller decides where to truncate.
    pub text: String,
}

impl RerankCandidate {
    /// Build one candidate at `rank`.
    pub fn new(id: impl Into<String>, rank: u32, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            rank,
            text: text.into(),
        }
    }
}

/// One scoring request, written to the child's stdin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RerankRequest {
    /// The protocol version the response must echo.
    pub protocol_version: u32,
    /// `rr-<yyyymmdd>-<8hex>`; the response must echo it.
    pub request_id: String,
    /// The immutable model identity the caller expects, echoed byte for byte.
    pub model: String,
    /// The adapter identity, `null` for the base model, echoed exactly.
    pub adapter: Option<String>,
    /// The search query, carried as data and never as a command.
    pub query: String,
    /// The candidates, ordered by `rank` ascending.
    pub candidates: Vec<RerankCandidate>,
}

impl RerankRequest {
    /// Build a request, minting `rr-<yyyymmdd>-<8hex>` from the query and
    /// `now`. Candidate `rank` values are the caller's; they are not rewritten.
    pub fn new(
        model: impl Into<String>,
        adapter: Option<String>,
        query: impl Into<String>,
        candidates: Vec<RerankCandidate>,
        now: OffsetDateTime,
    ) -> Self {
        let query = query.into();
        let request_id = dated_id(REQUEST_ID_PREFIX, &query, now);
        Self::with_request_id(request_id, model, adapter, query, candidates)
    }

    /// Build a request with a caller-supplied id, for a replayed trace or a
    /// test that must know the id before the child answers.
    pub fn with_request_id(
        request_id: impl Into<String>,
        model: impl Into<String>,
        adapter: Option<String>,
        query: impl Into<String>,
        candidates: Vec<RerankCandidate>,
    ) -> Self {
        Self {
            protocol_version: RERANK_PROTOCOL_VERSION,
            request_id: request_id.into(),
            model: model.into(),
            adapter,
            query: query.into(),
            candidates,
        }
    }

    /// The candidate ids in submitted order — the order to restore when a
    /// response is refused.
    pub fn submitted_order(&self) -> Vec<String> {
        self.candidates.iter().map(|c| c.id.clone()).collect()
    }
}

/// Whether a higher or a lower score means a better match. Declared by the
/// scorer on every response; never assumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoreDirection {
    /// The largest score ranks first.
    HigherIsBetter,
    /// The smallest score ranks first.
    LowerIsBetter,
}

/// One candidate's score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RerankScore {
    /// Must be one of the request's candidate ids.
    pub id: String,
    /// Must deserialize to a finite `f64`.
    pub score: f64,
}

/// One scoring response, read from the child's stdout.
///
/// `adapter` is an `Option`, so a response that omits the key deserializes as
/// `None`. That fails closed: a request expecting an adapter then sees an
/// identity mismatch rather than an unadapted score silently applied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RerankResponse {
    /// Must equal the request's.
    pub protocol_version: u32,
    /// Must equal the request's.
    pub request_id: String,
    /// Must equal the request's, byte for byte.
    pub model: String,
    /// Must equal the request's, `null` included.
    pub adapter: Option<String>,
    /// How to read `scores`.
    pub score_direction: ScoreDirection,
    /// Exactly one entry per request candidate; array order is irrelevant.
    pub scores: Vec<RerankScore>,
}

/// Validate the `rr-<yyyymmdd>-<8hex>` request-id shape.
pub fn is_valid_request_id(s: &str) -> bool {
    is_valid_dated_id(s, REQUEST_ID_PREFIX)
}

#[cfg(test)]
#[path = "tests/rerank_protocol.rs"]
mod tests;
