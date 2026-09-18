//! Run one bounded reranker child and apply only a fully valid response.
//!
//! The executable and its arguments come from trusted local configuration:
//! this type derives no `Deserialize`, so it can never be materialized from a
//! request body, and [`RerankRequest`] carries no command, argument, shell or
//! environment field. Query and candidate text reach the child only as bytes
//! on stdin, inside the JSON request.
//!
//! [`RerankRunner::rerank`] never returns `Err`: every operational failure is
//! a [`RerankOutcome::Declined`] carrying the submitted order.

use std::collections::HashSet;
use std::ffi::OsString;
use std::time::Duration;

use crate::utilities::process_runner::{
    ProcessError, ProcessFailure, ProcessLimits, ProcessOutput, ProcessRunner,
};
use crate::utilities::rerank_outcome::{
    RerankApplied, RerankDeclined, RerankFailure, RerankOutcome, RerankedCandidate,
};
use crate::utilities::rerank_protocol::{RerankRequest, RerankResponse};
use crate::utilities::rerank_validate::validate;

/// Default end-to-end budget for one rerank child.
///
/// Higher than the embed command's 10 s because a cross-encoder over a few
/// hundred candidates is not one short forward pass, and still low enough to
/// fail an interactive search fast. #213 makes it configurable when it wires a
/// search surface.
pub const DEFAULT_RERANK_TIMEOUT: Duration = Duration::from_secs(20);

/// Protocol-level bounds, on top of the byte bounds the process runner owns.
#[derive(Debug, Clone, Copy)]
pub struct RerankLimits {
    /// Largest candidate list accepted. The retrieval pool is capped by
    /// `COMEMORY_RETRIEVAL_MAX_PAGE_WINDOW` (200 by default), so this clears
    /// today's deepest pool with headroom.
    pub max_candidates: usize,
    /// Largest UTF-8 byte length of one candidate's text. Over-cap candidates
    /// are refused rather than truncated: where to cut is a semantic decision
    /// belonging to the candidate observation contract (#208).
    pub max_candidate_text_bytes: usize,
    /// Largest serialized request written to the child's stdin.
    pub max_request_bytes: usize,
    /// Largest response read back from the child's stdout.
    pub max_stdout_bytes: usize,
    /// Largest diagnostic stderr excerpt retained.
    pub max_stderr_bytes: usize,
}

impl Default for RerankLimits {
    fn default() -> Self {
        let process = ProcessLimits::default();
        Self {
            max_candidates: 256,
            max_candidate_text_bytes: 8 << 10,
            max_request_bytes: process.max_input_bytes,
            max_stdout_bytes: process.max_stdout_bytes,
            max_stderr_bytes: process.max_stderr_bytes,
        }
    }
}

/// The trusted local configuration that selects the scorer executable.
#[derive(Debug, Clone)]
pub struct RerankRunner {
    program: OsString,
    args: Vec<OsString>,
    timeout: Duration,
    limits: RerankLimits,
}

impl RerankRunner {
    /// Select the scorer executable and its arguments.
    pub fn new(program: impl Into<OsString>, args: Vec<OsString>) -> Self {
        Self {
            program: program.into(),
            args,
            timeout: DEFAULT_RERANK_TIMEOUT,
            limits: RerankLimits::default(),
        }
    }

    /// Replace the end-to-end budget.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Replace the protocol-level bounds.
    #[must_use]
    pub fn with_limits(mut self, limits: RerankLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Score one request and, only if the response fully validates, return the
    /// reranked order.
    pub fn rerank(&self, request: &RerankRequest) -> RerankOutcome {
        match self.attempt(request) {
            Ok(applied) => RerankOutcome::Applied(applied),
            Err((failure, stderr_excerpt)) => RerankOutcome::Declined(RerankDeclined {
                request_id: request.request_id.clone(),
                failure,
                original_order: request.submitted_order(),
                stderr_excerpt,
            }),
        }
    }

    /// The whole attempt, with the scorer's stderr excerpt carried alongside
    /// any failure so a caller can log what the child said about it.
    fn attempt(&self, request: &RerankRequest) -> Result<RerankApplied, (RerankFailure, String)> {
        check_request(request, self.limits).map_err(|f| (f, String::new()))?;
        let body =
            serde_json::to_vec(request).map_err(|e| (malformed(&e.to_string()), String::new()))?;
        let output = self.process_runner().run(&body).map_err(restate)?;
        let stderr_excerpt = String::from_utf8_lossy(&output.stderr).into_owned();
        apply(request, &output)
            .map_err(|failure| (failure, stderr_excerpt.clone()))
            .map(|order| RerankApplied {
                request_id: request.request_id.clone(),
                model: request.model.clone(),
                adapter: request.adapter.clone(),
                order,
                stderr_excerpt,
                elapsed: output.elapsed,
            })
    }

    /// The bounded process runner this configuration implies.
    fn process_runner(&self) -> ProcessRunner {
        ProcessRunner::new(self.program.clone(), self.args.clone())
            .with_timeout(self.timeout)
            .with_limits(ProcessLimits {
                max_input_bytes: self.limits.max_request_bytes,
                max_stdout_bytes: self.limits.max_stdout_bytes,
                max_stderr_bytes: self.limits.max_stderr_bytes,
            })
    }
}

/// Refuse a non-zero exit, then parse and validate the payload.
///
/// `ProcessOutput::input_truncated` is deliberately ignored, matching
/// `utilities::embed`: a scorer that answers from a canned payload without
/// draining stdin has made a choice, and its exit status and response still
/// decide the outcome. Nothing that reaches here can be wrong *because* the
/// request was not fully delivered — the response still has to echo the
/// request id, the model and the adapter, and score every candidate.
fn apply(
    request: &RerankRequest,
    output: &ProcessOutput,
) -> Result<Vec<RerankedCandidate>, RerankFailure> {
    if !output.status.success() {
        return Err(RerankFailure::NonZeroExit {
            code: output.status.code(),
        });
    }
    let response: RerankResponse =
        serde_json::from_slice(&output.stdout).map_err(|e| malformed(&e.to_string()))?;
    validate(request, &response)
}

/// Restate a process-level failure in the reranker's own vocabulary, keeping
/// the stderr the runner drained before it failed — for a scorer that times out
/// mid-load, that excerpt is the only diagnostic there is.
fn restate(error: ProcessError) -> (RerankFailure, String) {
    let excerpt = String::from_utf8_lossy(&error.stderr).into_owned();
    let failure = match error.failure {
        ProcessFailure::InputTooLarge { bytes, max } => {
            RerankFailure::RequestTooLarge { bytes, max }
        }
        ProcessFailure::Spawn(message) => RerankFailure::Spawn { message },
        ProcessFailure::Io { phase, message } => RerankFailure::Io {
            phase: phase.to_string(),
            message,
        },
        ProcessFailure::StdoutTooLarge { max } => RerankFailure::OutputTooLarge { max },
        ProcessFailure::TimedOut { budget } => RerankFailure::TimedOut {
            budget_ms: u64::try_from(budget.as_millis()).unwrap_or(u64::MAX),
        },
    };
    (failure, excerpt)
}

/// Refuse a request that cannot be scored before anything is spawned.
fn check_request(request: &RerankRequest, limits: RerankLimits) -> Result<(), RerankFailure> {
    if request.candidates.is_empty() {
        return Err(RerankFailure::EmptyCandidates);
    }
    if request.candidates.len() > limits.max_candidates {
        return Err(RerankFailure::TooManyCandidates {
            count: request.candidates.len(),
            max: limits.max_candidates,
        });
    }
    let mut seen: HashSet<&str> = HashSet::with_capacity(request.candidates.len());
    for candidate in &request.candidates {
        if !seen.insert(candidate.id.as_str()) {
            return Err(RerankFailure::DuplicateCandidateId {
                id: candidate.id.clone(),
            });
        }
        if candidate.text.len() > limits.max_candidate_text_bytes {
            return Err(RerankFailure::CandidateTextTooLarge {
                id: candidate.id.clone(),
                bytes: candidate.text.len(),
                max: limits.max_candidate_text_bytes,
            });
        }
    }
    Ok(())
}

/// The one construction of the malformed-response failure.
fn malformed(message: &str) -> RerankFailure {
    RerankFailure::Malformed {
        message: message.to_string(),
    }
}

#[cfg(test)]
#[path = "tests/rerank_runner.rs"]
mod tests;
