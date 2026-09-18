//! The dataset manifest (#210): what was exported, what was refused, how the
//! splits were assigned, and the two digests that make a repeat export
//! provable.
//!
//! The manifest carries no wall-clock field. A creation timestamp would make
//! two exports of one snapshot differ, and reproducibility is worth more than
//! a date each observation's `reference_time` already implies.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::domains::learning::evaluation::candidate_observation::RetrievalVersion;
use crate::domains::learning::evaluation::dataset_record::{LabelClass, RECORD_VERSION, Split};
use crate::domains::learning::evaluation::dataset_split::GroupAssignment;
use crate::store::candidate_dataset::DatasetSnapshot;
use crate::utilities::digest::sha256_hex;

/// Version of the manifest shape.
pub const MANIFEST_VERSION: u32 = 1;

/// How many hex characters of a digest become the dataset id.
const ID_HEX: usize = 16;

/// Everything the pipeline refused, dropped or left unlabelled, counted.
///
/// Every field is a report rather than a diagnostic: an export that drops a
/// record without a number here would be silently lossy, which is the failure
/// this whole design exists to prevent.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RowStats {
    /// Observations in the requested window.
    pub observations_scanned: u64,
    /// Observations written at an `observation_version` this build refuses.
    pub observations_refused_version: u64,
    /// Observations whose capture bound cut the pool. Their rows carry
    /// `pool_truncated`, and pool recall must not be computed from them.
    pub observations_truncated: u64,
    /// Observations dropped as duplicates of a kept one.
    pub duplicate_observations_dropped: u64,
    /// Candidates belonging to an admitted observation.
    pub candidates_scanned: u64,
    /// Candidates with no content snapshot. Never exported, labelled or not.
    pub candidates_unresolved: u64,
    /// Verdicts recorded against one of those. Reported, never exported.
    pub judgments_on_unresolved_candidates: u64,
    /// Candidates the `--domain` filter excluded.
    pub candidates_filtered_by_domain: u64,
    /// Verdicts on an admitted observation.
    pub judgments_scanned: u64,
    /// Verdicts naming a content version their observation did not see.
    pub judgments_stale: u64,
    /// Verdicts naming a candidate their observation never returned.
    pub judgments_recall_miss: u64,
    /// Verdicts a later verdict on the same query group revised away.
    pub judgments_contradictory_dropped: u64,
    /// Retrieved candidates nobody judged. The missing-label report.
    pub candidates_unjudged: u64,
    /// Reviewed negatives dropped by `--max-negatives-per-query`.
    pub negatives_capped: u64,
    /// Records written across every file, withheld files included.
    pub rows_emitted: u64,
}

/// The filtering configuration one export ran under.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterReport {
    /// `manual` | `implicit` | `all`.
    pub provenance: String,
    /// Whether unjudged candidates were emitted unlabelled.
    pub include_unjudged: bool,
    /// Whether the holdout split's files were written.
    pub include_holdout: bool,
    /// The domains in scope, in `CandidateDomain::all()` order and
    /// deduplicated — memory, then code, then document.
    pub domains: Vec<String>,
    /// Inclusive lower bound on `candidate_query_observations.at`.
    pub since: Option<String>,
    /// Exclusive upper bound on the same column.
    pub until: Option<String>,
    /// The reviewed-negative cap per query group per split; `0` is unlimited.
    pub max_negatives_per_query: usize,
}

/// The ratios the run REQUESTED. Assignment is a stable hash rather than a
/// shuffle, so the realized shares only approach these; `by_split` reports what
/// actually happened and is the truth.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SplitRatios {
    /// Requested training share.
    pub train: f64,
    /// Requested validation share.
    pub validation: f64,
    /// Requested holdout share.
    pub holdout: f64,
}

/// How the splits were assigned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SplitReport {
    /// The assignment policy. `grouped-hash` today.
    pub policy: String,
    /// The salt the group hash used.
    pub seed: String,
    /// The requested ratios.
    pub ratios: SplitRatios,
    /// The reserved repository, when one was named.
    pub holdout_repo: Option<String>,
    /// The reserved time slice, when one was named.
    pub holdout_since: Option<String>,
    /// How many connected components the rows formed.
    pub groups: usize,
    /// The largest component's row count. A number close to `rows_emitted`
    /// means this corpus cannot be split without leakage.
    pub largest_group_rows: usize,
    /// Every component, ascending by `group_id`.
    pub assignments: Vec<GroupAssignment>,
}

/// One label bucket. A label has two dimensions — its evidence class and its
/// grade — so a single-keyed map could not state both without ambiguity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelCount {
    /// `manual`, `implicit`, or `none` for an unjudged record.
    pub labels: String,
    /// The grade, or `null` for an unjudged record.
    pub relevance: Option<u8>,
    /// Records in this bucket.
    pub rows: u64,
}

/// One output file, written or withheld.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// The file's name inside the output directory.
    pub path: String,
    /// The split it holds.
    pub split: Split,
    /// Which evidence class it holds. Serialized as `manual` or `implicit`.
    pub labels: LabelClass,
    /// Records in it.
    pub rows: u64,
    /// Its byte length.
    pub bytes: u64,
    /// Its SHA-256. Present for a withheld file too: a digest proves a later
    /// qualification run scored the same rows without publishing them.
    pub sha256: String,
    /// Why the file was withheld, or `null` when it was written.
    pub reason: Option<String>,
}

/// The manifest itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetManifest {
    /// [`MANIFEST_VERSION`].
    pub manifest_version: u32,
    /// The record shape every emitted line was written at.
    pub record_version: u32,
    /// The observation contract every admitted capture was written at.
    pub observation_version: u32,
    /// `d-<16hex>` over the configuration and the input snapshot.
    pub dataset_id: String,
    /// `sha256` over every admitted input row.
    pub snapshot_digest: String,
    /// The binary that produced the export.
    pub binary_version: String,
    /// The applied schema version.
    pub schema_version: String,
    /// The filtering configuration.
    pub filters: FilterReport,
    /// The split configuration and its realized assignments.
    pub split: SplitReport,
    /// Everything refused, dropped or left unlabelled.
    pub counts: RowStats,
    /// Records per domain, ascending by domain.
    pub by_domain: BTreeMap<String, u64>,
    /// Records per split, ascending by split name.
    pub by_split: BTreeMap<String, u64>,
    /// Records per label bucket, ascending.
    pub by_label: Vec<LabelCount>,
    /// The full ranking and corpus version behind every record's
    /// `retrieval_revision`, ascending by `(knobs_hash, corpus.digest)`.
    pub retrieval_revisions: Vec<RetrievalVersion>,
    /// The files this export wrote.
    pub files: Vec<FileEntry>,
    /// The files it deliberately did not write.
    pub withheld: Vec<FileEntry>,
}

/// The identity of one input snapshot.
///
/// Candidate rows are in the digest because a purge redacts a passage IN PLACE
/// — blanking `text` and setting `unresolved` — without touching its
/// observation header, so a digest over headers alone would call two
/// materially different inputs the same snapshot.
pub fn snapshot_digest(snapshot: &DatasetSnapshot) -> String {
    let mut lines: Vec<String> = Vec::new();
    for o in &snapshot.observations {
        lines.push(format!(
            "o\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            o.observation_id,
            o.at,
            o.observation_version,
            o.knobs_hash,
            o.corpus_digest,
            o.candidate_count,
            o.truncated
        ));
    }
    for c in &snapshot.candidates {
        lines.push(format!(
            "c\t{}\t{}\t{}\t{}\t{}\t{}",
            c.observation_id,
            c.pool_position,
            c.candidate_ref,
            c.content_version,
            c.unresolved,
            c.text_sha256
        ));
    }
    for j in &snapshot.judgments {
        lines.push(format!(
            "j\t{}\t{}\t{}\t{}\t{}",
            j.observation_id, j.candidate_ref, j.relevance, j.provenance, j.at
        ));
    }
    sha256_hex(lines.join("\n").as_bytes())
}

/// `d-<16hex>` over the record version, the filtering configuration, the split
/// configuration and the input snapshot. Two runs sharing it wrote the same
/// bytes.
pub fn dataset_id(filters: &FilterReport, split: &SplitReport, snapshot: &str) -> String {
    let body = format!(
        "{MANIFEST_VERSION}\n{RECORD_VERSION}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{snapshot}",
        filters.provenance,
        filters.include_unjudged,
        filters.include_holdout,
        filters.domains.join(","),
        filters.since.as_deref().unwrap_or(""),
        filters.until.as_deref().unwrap_or(""),
        filters.max_negatives_per_query,
        split.policy,
        split.seed,
        split.ratios.train,
        split.ratios.validation,
        split.ratios.holdout,
        split.holdout_repo.as_deref().unwrap_or(""),
        split.holdout_since.as_deref().unwrap_or(""),
    );
    let digest = sha256_hex(body.as_bytes());
    format!("d-{}", digest.get(..ID_HEX).unwrap_or(digest.as_str()))
}

#[cfg(test)]
#[path = "tests/dataset_manifest.rs"]
mod tests;
