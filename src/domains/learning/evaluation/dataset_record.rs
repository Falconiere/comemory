//! The exported JSONL record: one line per `(observation, candidate)` pair
//! (#210).
//!
//! Versioned by [`RECORD_VERSION`], which a trainer must refuse when it does
//! not recognise it — the same rule `OBSERVATION_VERSION` imposes on a reader
//! of the persisted contract. No field here is a display value: identity and
//! `candidate_ref` are the matchable keys, the passage is what retrieval
//! matched on, and the locator never enters.

use serde::{Deserialize, Serialize};

use crate::domains::learning::evaluation::candidate_identity::CandidateIdentity;
use crate::domains::learning::evaluation::candidate_observation::EffectiveFilters;
use crate::utilities::telemetry::{PROV_IMPLICIT, PROV_MANUAL};

/// Version of the exported record shape. Bumped whenever a field is added,
/// removed, renamed or given new semantics.
pub const RECORD_VERSION: u32 = 1;

/// Which part of the dataset a record belongs to. `Holdout` is the
/// qualification split: it is withheld from an export unless it is asked for
/// by name, so training-time mining and model selection have no file to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Split {
    /// The training split.
    Train,
    /// The split a trainer may select a checkpoint on.
    Validation,
    /// The withheld qualification split.
    Holdout,
}

impl Split {
    /// The wire spelling, also the output file's stem.
    pub fn as_str(self) -> &'static str {
        match self {
            Split::Train => "train",
            Split::Validation => "validation",
            Split::Holdout => "holdout",
        }
    }

    /// Every split, in the order the ratios are declared and files written.
    pub fn all() -> [Split; 3] {
        [Split::Train, Split::Validation, Split::Holdout]
    }
}

/// Which evidence class a file carries. A reviewed verdict and an implicit
/// signal are never written to the same file, so an implicit label cannot be
/// promoted to evaluation truth by a trainer that globbed too widely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LabelClass {
    /// A human verdict: `candidate_judgments.provenance = 'manual'`.
    Manual,
    /// An observed signal: `candidate_judgments.provenance = 'implicit'`.
    Implicit,
}

impl LabelClass {
    /// The class a stored provenance word belongs to, or `None` for a word
    /// this build does not know.
    ///
    /// There is no inverse: a class renders itself through serde, whose
    /// `rename_all = "lowercase"` produces exactly these two words, so the
    /// spelling has one home rather than two that could drift.
    pub fn parse(provenance: &str) -> Option<LabelClass> {
        match provenance {
            PROV_MANUAL => Some(LabelClass::Manual),
            PROV_IMPLICIT => Some(LabelClass::Implicit),
            _ => None,
        }
    }
}

/// The verdict recorded on one candidate. Absent from a record whose candidate
/// nobody judged — never replaced by a zero, which is a reviewer's "not
/// relevant" and not a reviewer's silence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordLabel {
    /// Graded relevance, `0..=judgment::MAX_RELEVANCE`. `0` is a reviewed hard
    /// negative.
    pub relevance: u8,
    /// `manual` or `implicit`.
    pub provenance: String,
    /// When the verdict was recorded.
    pub at: String,
}

/// The ranking and corpus revision a record was produced under. The full
/// `RetrievalVersion` — every knob and the whole corpus snapshot — appears once
/// per distinct pair in the manifest, so a record stays small without losing
/// anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetrievalRevision {
    /// The binary that produced the observation.
    pub binary_version: String,
    /// The applied schema version.
    pub schema_version: String,
    /// The ranking configuration digest.
    pub knobs_hash: String,
    /// The corpus snapshot digest.
    pub corpus_digest: String,
}

/// One exported example: a query, a candidate retrieval produced for it, and
/// the verdict on that pair if one exists.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetRecord {
    /// [`RECORD_VERSION`] this line was written at.
    pub record_version: u32,
    /// The `OBSERVATION_VERSION` the underlying capture was written at.
    pub observation_version: u32,
    /// Which split this record belongs to.
    pub split: Split,
    /// The connected component of the query/content graph this record is in.
    /// A component never spans two splits.
    pub group_id: String,
    /// The near-duplicate key of `query`.
    pub query_group: String,
    /// The version-free, position-free key of this candidate's content.
    pub content_group: String,
    /// The captured observation this record came from.
    pub observation_id: String,
    /// The `retrieval_log.query_id` of the same run, when one was written.
    pub query_id: Option<String>,
    /// The query text, verbatim as retrieval received it.
    pub query: String,
    /// Which surface produced the run.
    pub source: String,
    /// The instant the run started: the contract's `reference_time`.
    pub reference_time: String,
    /// `memory` | `code` | `document`.
    pub domain: String,
    /// The domain-qualified reference string: the only matchable key.
    pub candidate_ref: String,
    /// The parsed form of `candidate_ref`, carried so a consumer need not
    /// implement the codec.
    pub identity: CandidateIdentity,
    /// The content version retrieval observed.
    pub content_version: String,
    /// The bounded passage retrieval matched on.
    pub text: String,
    /// Digest of the FULL passage, before bounding.
    pub text_sha256: String,
    /// Byte length of the full passage, before bounding.
    pub text_full_bytes: i64,
    /// Whether `text` is shorter than the full passage.
    pub text_truncated: bool,
    /// The verdict, or `null` when nobody judged this candidate.
    pub label: Option<RecordLabel>,
    /// 1-based position in the candidate pool, in retrieval's own order.
    pub pool_position: i64,
    /// 1-based position on the displayed page; `null` below the page cut.
    pub returned_position: Option<i64>,
    /// The fused score retrieval assigned.
    pub retrieval_score: f64,
    /// 1-based position within this candidate's own leg, before fusion.
    pub rank_in_domain: i64,
    /// Memory lexical-ladder tier; `null` for code and document candidates.
    pub tier: Option<i64>,
    /// Whether the capture bound cut this observation's pool. `true` forbids
    /// computing candidate-pool recall from this record's observation.
    pub pool_truncated: bool,
    /// Whether ACT-R activation was time-independent for the run.
    pub decay_frozen: bool,
    /// Every filter the run applied, per leg.
    pub filters: EffectiveFilters,
    /// The ranking and corpus revision the run scored against.
    pub retrieval_revision: RetrievalRevision,
}
