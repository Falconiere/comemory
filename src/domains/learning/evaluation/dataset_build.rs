//! Assembling split rows into the files a dataset export writes (#210).
//!
//! Pure: no filesystem, no database. Its one ordering guarantee is what makes
//! the qualification split provably unreachable — rows are bucketed by
//! `(split, label class)` FIRST, and the reviewed-negative cap then runs inside
//! one bucket, so no selection step can see a row from another split.

use std::collections::BTreeMap;

use crate::domains::learning::evaluation::candidate_observation::OBSERVATION_VERSION;
use crate::domains::learning::evaluation::candidate_observation::RetrievalVersion;
use crate::domains::learning::evaluation::dataset_manifest::{LabelCount, RowStats};
use crate::domains::learning::evaluation::dataset_record::{
    DatasetRecord, LabelClass, RECORD_VERSION, RetrievalRevision, Split,
};
use crate::domains::learning::evaluation::dataset_rows::{BuiltRows, ObservationContext};
use crate::domains::learning::evaluation::dataset_select::PendingRow;
use crate::domains::learning::evaluation::dataset_split::SplitPlan;

/// The `labels` word an unjudged record is counted under in the manifest.
const LABELS_NONE: &str = "none";

/// Which file a record belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BucketKey {
    /// The split.
    pub split: Split,
    /// The evidence class. Unjudged records ride in the `Manual` bucket, the
    /// primary file, because they carry no evidence class of their own.
    pub labels: LabelClass,
}

impl BucketKey {
    /// The file name this bucket writes to.
    pub fn file_name(self) -> String {
        match self.labels {
            LabelClass::Manual => format!("{}.jsonl", self.split.as_str()),
            LabelClass::Implicit => format!("{}.implicit.jsonl", self.split.as_str()),
        }
    }
}

/// What one assembly reads.
pub struct BuildInput<'a> {
    /// The rows and their observation contexts.
    pub built: &'a BuiltRows,
    /// Each row's component and split, in `built.rows` order.
    pub plan: &'a SplitPlan,
    /// Which evidence classes this run writes files for.
    pub classes: &'a [LabelClass],
    /// Reviewed-negative cap per query group per bucket; `0` is unlimited.
    pub max_negatives_per_query: usize,
}

/// What one assembly produced.
pub struct Dataset {
    /// One entry per file this run writes, ascending by key.
    pub buckets: Vec<(BucketKey, Vec<DatasetRecord>)>,
    /// Records per domain.
    pub by_domain: BTreeMap<String, u64>,
    /// Records per split.
    pub by_split: BTreeMap<String, u64>,
    /// Records per label bucket.
    pub by_label: Vec<LabelCount>,
    /// The full ranking and corpus versions behind those records.
    pub revisions: Vec<RetrievalVersion>,
}

/// Bucket, cap and order every row, then count what came out.
pub fn assemble(input: BuildInput<'_>, stats: &mut RowStats) -> Dataset {
    let mut buckets: BTreeMap<BucketKey, Vec<DatasetRecord>> = BTreeMap::new();
    for split in Split::all() {
        for labels in [LabelClass::Manual, LabelClass::Implicit] {
            // The Manual bucket is the PRIMARY file and exists for every split
            // whatever the provenance filter says, because an unjudged record
            // carries no evidence class and has nowhere else to go. Only the
            // implicit file is conditional.
            if labels == LabelClass::Manual || input.classes.contains(&labels) {
                buckets.insert(BucketKey { split, labels }, Vec::new());
            }
        }
    }
    for (row, (group_id, split)) in input.built.rows.iter().zip(&input.plan.per_row) {
        let Some(ctx) = input.built.contexts.get(row.observation) else {
            continue;
        };
        let key = BucketKey {
            split: *split,
            labels: row.label_class.unwrap_or(LabelClass::Manual),
        };
        if let Some(bucket) = buckets.get_mut(&key) {
            bucket.push(record_of(row, ctx, group_id, *split));
        }
    }
    for records in buckets.values_mut() {
        records.sort_by(|a, b| {
            (&a.group_id, &a.observation_id, a.pool_position).cmp(&(
                &b.group_id,
                &b.observation_id,
                b.pool_position,
            ))
        });
        stats.negatives_capped = stats
            .negatives_capped
            .saturating_add(cap_negatives(records, input.max_negatives_per_query));
    }
    let buckets: Vec<(BucketKey, Vec<DatasetRecord>)> = buckets.into_iter().collect();
    stats.rows_emitted = buckets.iter().map(|(_, r)| r.len() as u64).sum();
    Dataset {
        by_domain: tally(&buckets, |r| r.domain.clone()),
        by_split: tally(&buckets, |r| r.split.as_str().to_string()),
        by_label: label_counts(&buckets),
        revisions: revisions(&input.built.contexts),
        buckets,
    }
}

/// Drop reviewed relevance-`0` records beyond `cap` for one query group,
/// keeping the ones retrieval ranked highest. Returns how many were dropped.
///
/// `cap` of `0` is unlimited. This runs inside one bucket, after the split has
/// already been assigned, which is what "splits before negative mining" means
/// mechanically rather than aspirationally.
fn cap_negatives(records: &mut Vec<DatasetRecord>, cap: usize) -> u64 {
    if cap == 0 {
        return 0;
    }
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let before = records.len();
    records.retain(|record| {
        if record.label.as_ref().is_none_or(|l| l.relevance != 0) {
            return true;
        }
        let count = seen.entry(record.query_group.clone()).or_default();
        *count = count.saturating_add(1);
        *count <= cap
    });
    before.saturating_sub(records.len()) as u64
}

/// One record.
fn record_of(
    row: &PendingRow,
    ctx: &ObservationContext,
    group_id: &str,
    split: Split,
) -> DatasetRecord {
    DatasetRecord {
        record_version: RECORD_VERSION,
        observation_version: OBSERVATION_VERSION,
        split,
        group_id: group_id.to_string(),
        query_group: ctx.query_group.clone(),
        content_group: row.content_group.clone(),
        observation_id: ctx.observation_id.clone(),
        query_id: ctx.query_id.clone(),
        query: ctx.query.clone(),
        source: ctx.source.clone(),
        reference_time: ctx.at.clone(),
        domain: row.identity.domain().as_str().to_string(),
        candidate_ref: row.candidate_ref.clone(),
        identity: row.identity.clone(),
        content_version: row.content_version.clone(),
        text: row.text.clone(),
        text_sha256: row.text_sha256.clone(),
        text_full_bytes: row.text_full_bytes,
        text_truncated: row.text_truncated,
        label: row.label.clone(),
        pool_position: row.pool_position,
        returned_position: row.returned_position,
        retrieval_score: row.retrieval_score,
        rank_in_domain: row.rank_in_domain,
        tier: row.tier,
        pool_truncated: ctx.truncated,
        decay_frozen: ctx.decay_frozen,
        filters: ctx.filters.clone(),
        retrieval_revision: RetrievalRevision {
            binary_version: ctx.revision.binary_version.clone(),
            schema_version: ctx.revision.schema_version.clone(),
            knobs_hash: ctx.revision.knobs_hash.clone(),
            corpus_digest: ctx.revision.corpus_digest.clone(),
        },
    }
}

/// Count records by a derived key, ascending.
fn tally(
    buckets: &[(BucketKey, Vec<DatasetRecord>)],
    key: impl Fn(&DatasetRecord) -> String,
) -> BTreeMap<String, u64> {
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    for record in buckets.iter().flat_map(|(_, records)| records) {
        let slot = counts.entry(key(record)).or_default();
        *slot = slot.saturating_add(1);
    }
    counts
}

/// Records per `(evidence class, grade)` bucket, ascending.
fn label_counts(buckets: &[(BucketKey, Vec<DatasetRecord>)]) -> Vec<LabelCount> {
    let mut counts: BTreeMap<(String, Option<u8>), u64> = BTreeMap::new();
    for record in buckets.iter().flat_map(|(_, records)| records) {
        let key = match &record.label {
            Some(label) => (label.provenance.clone(), Some(label.relevance)),
            None => (LABELS_NONE.to_string(), None),
        };
        let slot = counts.entry(key).or_default();
        *slot = slot.saturating_add(1);
    }
    counts
        .into_iter()
        .map(|((labels, relevance), rows)| LabelCount {
            labels,
            relevance,
            rows,
        })
        .collect()
}

/// Every distinct ranking and corpus version, ascending by
/// `(knobs_hash, corpus.digest)`.
fn revisions(contexts: &[ObservationContext]) -> Vec<RetrievalVersion> {
    let mut by_key: BTreeMap<(String, String), RetrievalVersion> = BTreeMap::new();
    for ctx in contexts {
        by_key.insert(
            (
                ctx.retrieval.knobs_hash.clone(),
                ctx.retrieval.corpus.digest.clone(),
            ),
            ctx.retrieval.clone(),
        );
    }
    by_key.into_values().collect()
}
