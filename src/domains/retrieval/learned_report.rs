//! What a learned ordering stage did to one requested search, as a caller sees
//! it (#213).
//!
//! Serialized under the `learned` key of every retrieval payload. Each payload
//! spells "no stage ran" the way it already spells an absent field: `search`,
//! `search-code` and `context` share an envelope whose optional fields are
//! skipped, so the key is absent entirely and their JSON is byte-identical to a
//! build without this; `find` and the console search assemble their objects by
//! hand and spell absent fields as explicit nulls, as they already do for
//! `query_id`, so there it is present and null.
//!
//! The four things the issue asks to be distinct are distinct fields here: the
//! learned score ([`LearnedScore::score`]), the effective order
//! ([`LearnedScore::rank`], beside the deterministic order it replaced), the
//! model identity ([`LearnedOrdering::model`] / [`LearnedOrdering::adapter`]),
//! and the fallback status ([`LearnedOrdering::applied`] /
//! [`LearnedOrdering::fallback`]). None of them touches a hit's deterministic
//! `score_parts`, which stays exactly what the deterministic ranking produced.

use serde::Serialize;

/// One scored candidate's place in the learned order.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LearnedScore {
    /// The opaque, domain-qualified id that was on the wire: `<domain>:<id>`,
    /// the key the candidate pool is keyed by and therefore unique within one
    /// request.
    pub candidate_id: String,
    /// The candidate observation contract's reference string, carrying the
    /// stable identity and the content version. Reported for a reader, never
    /// matched on: unlike `candidate_id` it is not unique within a pool —
    /// two same-named functions in one file share one.
    pub candidate_ref: String,
    /// 1-based position in the learned order that was applied.
    pub rank: usize,
    /// 1-based position in the deterministic order that was submitted.
    ///
    /// Carried so the ranking a machine-dependent prior produced is visible in
    /// the response rather than invisible: two machines whose code working sets
    /// disagree submit different orders, and this is where that shows.
    pub deterministic_rank: usize,
    /// The finite score the scorer returned.
    pub score: f64,
}

/// One requested search's learned ordering stage, applied or refused.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LearnedOrdering {
    /// Whether the scorer's response validated and was applied. When `false`
    /// the entire deterministic candidate order stands, unchanged.
    pub applied: bool,
    /// The model identity that was asked for, which a valid response echoed
    /// byte for byte.
    pub model: String,
    /// The adapter identity that was asked for; absent for the base model.
    pub adapter: Option<String>,
    /// `rr-<yyyymmdd>-<8hex>` of this run's scoring request.
    pub request_id: String,
    /// How many candidates the deterministic ranking produced — the bounded
    /// candidate universe, independent of the requested page.
    pub pool: usize,
    /// How many leading candidates were submitted for scoring. The remaining
    /// `pool - prefix` keep their deterministic order exactly.
    pub prefix: usize,
    /// Wall-clock time of the scorer child, in milliseconds. `0` when the
    /// response was refused: a refusal carries no measured duration, not even
    /// for a run that burned its whole budget.
    pub elapsed_ms: u64,
    /// Why the deterministic order stands, present only when `applied` is
    /// `false`. The scorer's own refusal message, verbatim.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
    /// The learned order, one entry per submitted candidate, ascending by
    /// [`LearnedScore::rank`]. Empty on a refusal, because nothing was scored.
    pub scores: Vec<LearnedScore>,
}

impl LearnedOrdering {
    /// A single human-readable line for the TTY renderers, which have no room
    /// for the score table.
    ///
    /// Shared by all four surfaces so `search`, `search-code`, `find` and
    /// `context` cannot drift on how a learned run announces itself.
    pub fn summary(&self) -> String {
        let identity = match &self.adapter {
            Some(a) => format!("{} ({a})", self.model),
            None => self.model.clone(),
        };
        match &self.fallback {
            Some(why) => format!("learned: {identity}  fallback: {why}"),
            None => format!(
                "learned: {identity}  prefix {}/{}  {}ms",
                self.prefix, self.pool, self.elapsed_ms
            ),
        }
    }
}

#[cfg(test)]
#[path = "tests/learned_report.rs"]
mod tests;
