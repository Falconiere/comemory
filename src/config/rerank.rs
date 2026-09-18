//! `[rerank]` config section: the opt-in learned ordering stage (#213).
//!
//! Its own file beside `observations.rs` for the reason that one has one — a
//! section owns its struct, its file overlay and its invariants, and `file.rs`
//! and `validate.rs` stay at one line each.
//!
//! **File-only, like `[tune]`.** There are no `COMEMORY_RERANK_*` overrides. A
//! command vector has no readable single-string environment encoding, and the
//! remaining knobs are machine configuration an operator sets once rather than
//! per invocation. Rollback is `enabled = false`, or pointing `command` at a
//! backend invoked without an adapter.

use serde::{Deserialize, Serialize};

use crate::prelude::*;
use crate::utilities::rerank_runner::RerankLimits;

/// Shipped default for [`RerankConfig::prefix`] — how many leading candidates
/// one request scores.
const DEFAULT_PREFIX: usize = 50;

/// Shipped default for [`RerankConfig::timeout_ms`], matching
/// `utilities::rerank_runner::DEFAULT_RERANK_TIMEOUT`.
const DEFAULT_TIMEOUT_MS: u64 = 20_000;

/// Shipped default for [`RerankConfig::max_candidate_text_bytes`], matching the
/// candidate observation contract's own text bound.
const DEFAULT_CANDIDATE_TEXT_BYTES: usize = 4096;

/// Ceiling the serialized scorer request is held to.
///
/// DERIVED from the process utilities rather than restated, so the two cannot
/// drift: a restated constant is only as good as the test that compares it, and
/// the comparison would pass at test time while a release shipped the drift.
/// `config::defaults` already reaches into `utilities` the same way for
/// `simhash::NEAR_DUP_HAMMING`.
pub fn max_request_bytes() -> usize {
    RerankLimits::default().max_request_bytes
}

/// The optional learned ordering stage: which scorer to run, how much of the
/// ranking to hand it, and what it is allowed to cost.
///
/// Off by default. When it is off nothing is constructed, no child process is
/// launched, and the deterministic ranking is returned byte for byte.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankConfig {
    /// Whether a learned ordering stage runs at all. Default `false`.
    pub enabled: bool,
    /// Program and arguments for the scorer, passed to `Command` separately
    /// and never assembled into a shell line. Its first entry is the program
    /// and is validated non-blank whenever `enabled` is set.
    pub command: Vec<String>,
    /// The immutable model identity the response must echo byte for byte.
    /// Validated non-blank whenever `enabled` is set.
    pub model: String,
    /// The adapter identity, absent for the base model. An empty or blank
    /// string in the file reads as absent, so clearing the key is a rollback
    /// to base scores rather than a request for an adapter named "".
    pub adapter: Option<String>,
    /// How many leading candidates of the deterministic ranking are scored.
    /// The tail below it is preserved exactly. Validated `>= 1`.
    pub prefix: usize,
    /// End-to-end budget for one scorer child, in milliseconds. Validated
    /// `>= 1`: a zero budget would refuse every run, which `enabled = false`
    /// already says more clearly.
    pub timeout_ms: u64,
    /// Per-candidate text bound handed to the candidate observation contract's
    /// `BoundedText::bound`. Validated `>= 1`.
    pub max_candidate_text_bytes: usize,
}

impl Default for RerankConfig {
    /// Off, with no command and no model, at bounds that keep an enabled stage
    /// inside one request of the process runner's default 8 MiB stdin ceiling.
    fn default() -> Self {
        Self {
            enabled: false,
            command: Vec::new(),
            model: String::new(),
            adapter: None,
            prefix: DEFAULT_PREFIX,
            timeout_ms: DEFAULT_TIMEOUT_MS,
            max_candidate_text_bytes: DEFAULT_CANDIDATE_TEXT_BYTES,
        }
    }
}

impl RerankConfig {
    /// Overlay the file's `[rerank]` keys; absent keys leave `self`.
    pub(crate) fn apply(&mut self, p: PartialRerankConfig) {
        if let Some(v) = p.enabled {
            self.enabled = v;
        }
        if let Some(v) = p.command {
            self.command = v;
        }
        if let Some(v) = p.model {
            self.model = v;
        }
        if let Some(v) = p.adapter {
            self.adapter = blank_to_none(v);
        }
        if let Some(v) = p.prefix {
            self.prefix = v;
        }
        if let Some(v) = p.timeout_ms {
            self.timeout_ms = v;
        }
        if let Some(v) = p.max_candidate_text_bytes {
            self.max_candidate_text_bytes = v;
        }
    }

    /// The adapter identity this configuration asks for, absent for base.
    pub fn adapter(&self) -> Option<&str> {
        self.adapter.as_deref()
    }

    /// Every invariant, whether or not the stage is enabled.
    ///
    /// The three bounds are checked unconditionally so a typo is reported when
    /// it is written rather than the first time someone flips `enabled`. The
    /// two identity keys are checked only when enabled, because an operator who
    /// has turned the stage off is entitled to leave them empty.
    pub(crate) fn validate(&self) -> Result<()> {
        if self.prefix < 1 {
            return Err(invalid("prefix", self.prefix, "must be >= 1"));
        }
        if self.timeout_ms < 1 {
            return Err(invalid(
                "timeout_ms",
                self.timeout_ms,
                "must be >= 1 (use rerank.enabled = false to disable the stage)",
            ));
        }
        if self.max_candidate_text_bytes < 1 {
            return Err(invalid(
                "max_candidate_text_bytes",
                self.max_candidate_text_bytes,
                "must be >= 1",
            ));
        }
        // `checked_mul`, not `saturating_mul`: a product that overflows `usize`
        // is a misconfiguration in its own right and is reported as one, rather
        // than arriving at the comparison as a capped value that happens to
        // fail it for the wrong reason.
        let limit = max_request_bytes();
        let over = match self.prefix.checked_mul(self.max_candidate_text_bytes) {
            Some(bytes) if bytes <= limit => None,
            Some(bytes) => Some(bytes.to_string()),
            None => Some("more than usize::MAX".to_string()),
        };
        if let Some(request_bytes) = over {
            return Err(Error::Config(format!(
                "invalid rerank.prefix={} × rerank.max_candidate_text_bytes={} (file-only [rerank] keys): \
                 their product of {request_bytes} bytes exceeds the {limit}-byte scorer request limit",
                self.prefix, self.max_candidate_text_bytes
            )));
        }
        self.validate_identity()
    }

    /// The two keys an enabled stage cannot run without.
    fn validate_identity(&self) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        // The FIRST element is the program; a blank one spawns nothing, and
        // checking only "some element is non-blank" would accept
        // `["", "score.py"]` and defer the failure to a `Spawn` refusal.
        if self
            .command
            .first()
            .is_none_or(|program| program.trim().is_empty())
        {
            return Err(Error::Config(
                "invalid rerank.command (file-only [rerank] key): the first entry must name a program when rerank.enabled is true"
                    .into(),
            ));
        }
        if self.model.trim().is_empty() {
            return Err(Error::Config(
                "invalid rerank.model=\"\" (file-only [rerank] key): must name the model identity the scorer echoes when rerank.enabled is true"
                    .into(),
            ));
        }
        Ok(())
    }
}

/// A blank adapter string is "no adapter", not an adapter whose name is empty.
fn blank_to_none(v: String) -> Option<String> {
    if v.trim().is_empty() { None } else { Some(v) }
}

/// The one construction of a `[rerank]` bound failure. No env var is cited
/// because the section has none.
fn invalid(field: &str, value: impl std::fmt::Display, why: &str) -> Error {
    Error::Config(format!(
        "invalid rerank.{field}={value} (file-only [rerank] key): {why}"
    ))
}

/// File-overlay partial for [`RerankConfig`].
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct PartialRerankConfig {
    pub(crate) enabled: Option<bool>,
    pub(crate) command: Option<Vec<String>>,
    pub(crate) model: Option<String>,
    pub(crate) adapter: Option<String>,
    pub(crate) prefix: Option<usize>,
    pub(crate) timeout_ms: Option<u64>,
    pub(crate) max_candidate_text_bytes: Option<usize>,
}

#[cfg(test)]
#[path = "tests/rerank.rs"]
mod tests;
