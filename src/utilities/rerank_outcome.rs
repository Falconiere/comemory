//! The reranker result vocabulary: the applied order and every typed refusal.
//!
//! [`RerankFailure`] is deliberately not a [`crate::Error`] variant. A scorer
//! that fails must never be able to become a search failure through a stray
//! `?`; the caller reads [`RerankOutcome::order_ids`] in both arms and carries
//! on with the deterministic ranking it already had.

use std::time::Duration;

use thiserror::Error;

/// One candidate in the order the caller should now use.
#[derive(Debug, Clone, PartialEq)]
pub struct RerankedCandidate {
    /// The opaque candidate id, exactly as submitted.
    pub id: String,
    /// The candidate's submitted rank, which is also the tie-break key.
    pub rank: u32,
    /// The finite score the scorer returned.
    pub score: f64,
}

/// Every way a rerank attempt can be refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RerankFailure {
    /// The request carried no candidates.
    #[error("no candidates to rerank")]
    EmptyCandidates,
    /// More candidates than the configured ceiling.
    #[error("{count} candidates exceed the limit of {max}")]
    TooManyCandidates {
        /// How many were offered.
        count: usize,
        /// The configured ceiling.
        max: usize,
    },
    /// Two candidates share an id, so a score could not be attributed.
    #[error("duplicate candidate id {id}")]
    DuplicateCandidateId {
        /// The repeated id.
        id: String,
    },
    /// One candidate's text is larger than the configured ceiling.
    #[error("candidate {id} text of {bytes} bytes exceeds the limit of {max}")]
    CandidateTextTooLarge {
        /// The offending candidate.
        id: String,
        /// Its text size in bytes.
        bytes: usize,
        /// The configured ceiling.
        max: usize,
    },
    /// The serialized request is larger than the configured ceiling.
    #[error("request of {bytes} bytes exceeds the limit of {max}")]
    RequestTooLarge {
        /// The serialized size.
        bytes: usize,
        /// The configured ceiling.
        max: usize,
    },
    /// The scorer process could not be started.
    #[error("reranker spawn failed: {message}")]
    Spawn {
        /// The operating-system message.
        message: String,
    },
    /// A pipe or wait operation failed.
    #[error("reranker {phase}: {message}")]
    Io {
        /// Which step failed.
        phase: String,
        /// The operating-system message.
        message: String,
    },
    /// The end-to-end budget expired.
    #[error("reranker timed out after {budget_ms}ms")]
    TimedOut {
        /// The budget that was exceeded, in milliseconds.
        budget_ms: u64,
    },
    /// The scorer exited non-zero, or was killed by a signal (`code: None`).
    #[error("reranker exited with {code:?}")]
    NonZeroExit {
        /// The exit code, absent when a signal ended the process.
        code: Option<i32>,
    },
    /// The scorer wrote more than the stdout ceiling allows.
    #[error("reranker output exceeds the limit of {max} bytes")]
    OutputTooLarge {
        /// The configured ceiling.
        max: usize,
    },
    /// The response is not a JSON object of the declared shape.
    #[error("malformed reranker response: {message}")]
    Malformed {
        /// The parser's own message.
        message: String,
    },
    /// The response declares a different protocol version.
    #[error("protocol version {actual} does not match the requested {expected}")]
    VersionMismatch {
        /// What the request declared.
        expected: u32,
        /// What the response declared.
        actual: u32,
    },
    /// The response echoes a different request id.
    #[error("request id {actual} does not match {expected}")]
    RequestIdMismatch {
        /// The id that was sent.
        expected: String,
        /// The id that came back.
        actual: String,
    },
    /// The response was produced by a different model than was asked for.
    #[error("model {actual} does not match the expected {expected}")]
    ModelMismatch {
        /// The identity that was asked for.
        expected: String,
        /// The identity that answered.
        actual: String,
    },
    /// The response was produced with a different adapter.
    #[error("adapter {actual:?} does not match the expected {expected:?}")]
    AdapterMismatch {
        /// The identity that was asked for.
        expected: Option<String>,
        /// The identity that answered.
        actual: Option<String>,
    },
    /// One or more candidates were not scored.
    #[error("no score for {} candidate(s)", ids.len())]
    MissingScores {
        /// The unscored candidate ids, in submitted order.
        ids: Vec<String>,
    },
    /// One candidate was scored more than once.
    #[error("candidate {id} scored more than once")]
    DuplicateScore {
        /// The repeated id.
        id: String,
    },
    /// A score names a candidate that was never offered.
    #[error("score for unknown candidate {id}")]
    UnknownScore {
        /// The unrecognized id.
        id: String,
    },
    /// A score is `NaN` or infinite.
    #[error("non-finite score for candidate {id}")]
    NonFiniteScore {
        /// The offending candidate.
        id: String,
    },
}

/// A response that fully validated, with the identity the scorer confirmed.
#[derive(Debug, Clone)]
pub struct RerankApplied {
    /// The request id both sides agreed on.
    pub request_id: String,
    /// The model identity the scorer confirmed.
    pub model: String,
    /// The adapter identity the scorer confirmed.
    pub adapter: Option<String>,
    /// The candidates in the order to use now.
    pub order: Vec<RerankedCandidate>,
    /// The scorer's retained stderr excerpt, as text.
    pub stderr_excerpt: String,
    /// End-to-end wall-clock time of the child run.
    pub elapsed: Duration,
}

/// Nothing was applied, and why.
#[derive(Debug, Clone)]
pub struct RerankDeclined {
    /// The request id that was sent.
    pub request_id: String,
    /// What went wrong.
    pub failure: RerankFailure,
    /// Every submitted candidate id, in submitted rank order, so the complete
    /// original ranking can be restored from this value alone.
    pub original_order: Vec<String>,
    /// The scorer's retained stderr excerpt, as text.
    pub stderr_excerpt: String,
}

/// The result of one rerank attempt.
#[derive(Debug, Clone)]
pub enum RerankOutcome {
    /// The response validated; use [`RerankApplied::order`].
    Applied(RerankApplied),
    /// The response was refused; the original order stands.
    Declined(RerankDeclined),
}

impl RerankOutcome {
    /// The candidate ids in the order the caller should use — the reranked
    /// order when applied, the submitted order when declined.
    pub fn order_ids(&self) -> Vec<&str> {
        match self {
            Self::Applied(applied) => applied.order.iter().map(|c| c.id.as_str()).collect(),
            Self::Declined(declined) => {
                declined.original_order.iter().map(String::as_str).collect()
            }
        }
    }

    /// Whether a response was applied.
    pub fn is_applied(&self) -> bool {
        matches!(self, Self::Applied(_))
    }

    /// The refusal, when there was one.
    pub fn failure(&self) -> Option<&RerankFailure> {
        match self {
            Self::Applied(_) => None,
            Self::Declined(declined) => Some(&declined.failure),
        }
    }
}

#[cfg(test)]
#[path = "tests/rerank_outcome.rs"]
mod tests;
