//! Which candidates and which verdicts survive into the dataset, and the rows
//! they become (#210).
//!
//! Every rule here is a drop with a name: an unresolved candidate, a domain
//! this run excludes, a duplicate observation, a verdict that names a content
//! version the observation did not see, a verdict that names nothing it
//! returned, and a verdict a later one revised. Nothing is invented — a
//! candidate nobody judged stays unlabelled.

use std::collections::{BTreeMap, BTreeSet};

use crate::domains::learning::evaluation::candidate_identity::{
    CandidateDomain, CandidateIdentity, parse_ref,
};
use crate::domains::learning::evaluation::dataset_dedup::{
    ContradictionInput, VerdictOutcome, classify, resolve_contradictions,
};
use crate::domains::learning::evaluation::dataset_manifest::RowStats;
use crate::domains::learning::evaluation::dataset_record::{LabelClass, RecordLabel};
use crate::domains::learning::evaluation::dataset_rows::{ObservationContext, RowInput};
use crate::domains::learning::evaluation::dataset_split::content_group;
use crate::domains::learning::evaluation::judgment::MAX_RELEVANCE;
use crate::prelude::*;
use crate::store::candidate_dataset::{DatasetCandidate, DatasetJudgment, DatasetSnapshot};

/// One resolved candidate of one observation, borrowed from the snapshot.
pub struct Resolved<'a> {
    /// The parsed identity of [`Self::row`].
    pub identity: CandidateIdentity,
    /// Whether this run's domain filter admits it.
    pub in_domain: bool,
    /// The stored row.
    pub row: &'a DatasetCandidate,
}

/// One verdict that matched a candidate its own observation returned.
pub struct Verdict {
    /// The observation slot it belongs to.
    pub observation: usize,
    /// The candidate slot inside that observation.
    pub candidate: usize,
    /// The reference of the candidate it matched, as that observation recorded
    /// it. Carried because a candidate slot is only meaningful inside one
    /// observation, while a contradiction is a disagreement ACROSS them.
    pub candidate_ref: String,
    /// The verdict as a record carries it.
    pub label: RecordLabel,
    /// Which file the labelled row goes to.
    pub class: LabelClass,
}

/// One candidate row before it has a group or a split.
pub struct PendingRow {
    /// Index into `BuiltRows::contexts`.
    pub observation: usize,
    /// The parsed identity.
    pub identity: CandidateIdentity,
    /// The domain-qualified reference string: the only matchable key.
    pub candidate_ref: String,
    /// The version-free content key.
    pub content_group: String,
    /// The content version retrieval observed.
    pub content_version: String,
    /// The bounded passage.
    pub text: String,
    /// Digest of the full passage, before bounding.
    pub text_sha256: String,
    /// Byte length of the full passage, before bounding.
    pub text_full_bytes: i64,
    /// Whether the passage was bounded.
    pub text_truncated: bool,
    /// 1-based pool position.
    pub pool_position: i64,
    /// 1-based page position, when it was on the page.
    pub returned_position: Option<i64>,
    /// The fused score retrieval assigned.
    pub retrieval_score: f64,
    /// 1-based position in the candidate's own leg.
    pub rank_in_domain: i64,
    /// Memory lexical-ladder tier, where the domain has one.
    pub tier: Option<i64>,
    /// The verdict, absent when nobody judged this candidate.
    pub label: Option<RecordLabel>,
    /// The evidence class of the label, and so which file this row goes to.
    pub label_class: Option<LabelClass>,
    /// The repo label of a code candidate, for the reserved-repository rule.
    pub repo: Option<String>,
}

/// Every row of `rows` whose observation was admitted, paired with its slot.
///
/// Both walks below range over a snapshot table keyed by `observation_id` and
/// must skip a row whose observation this build refused, so the skip lives
/// here instead of being restated at each call site.
fn admitted<'a, T: 'a>(
    rows: &'a [T],
    index: &'a BTreeMap<String, usize>,
    id: impl Fn(&T) -> &str + 'a,
) -> impl Iterator<Item = (usize, &'a T)> + 'a {
    rows.iter()
        .filter_map(move |row| index.get(id(row)).map(|slot| (*slot, row)))
}

/// Every candidate of every admitted observation, resolved to its identity and
/// flagged for the domain filter. Unresolved candidates are dropped here: a
/// candidate with no content snapshot can never be judged and can never be
/// training data, whatever a verdict against it says.
pub fn resolve<'a>(
    snapshot: &'a DatasetSnapshot,
    index: &'a BTreeMap<String, usize>,
    domains: &[CandidateDomain],
    stats: &mut RowStats,
) -> Result<Vec<Vec<Resolved<'a>>>> {
    let mut per_observation: Vec<Vec<Resolved<'a>>> =
        (0..index.len()).map(|_| Vec::new()).collect();
    for (slot, candidate) in admitted(&snapshot.candidates, index, |c| &c.observation_id) {
        stats.candidates_scanned = stats.candidates_scanned.saturating_add(1);
        let Some(entry) = resolved_one(candidate, domains, stats)? else {
            continue;
        };
        if let Some(bucket) = per_observation.get_mut(slot) {
            bucket.push(entry);
        }
    }
    Ok(per_observation)
}

/// One candidate's resolution, or `None` when it has no content snapshot.
///
/// The domain filter only FLAGS: an excluded candidate stays in the list so a
/// verdict naming it can be recognised as filtered rather than miscounted as a
/// candidate-pool recall miss.
fn resolved_one<'a>(
    candidate: &'a DatasetCandidate,
    domains: &[CandidateDomain],
    stats: &mut RowStats,
) -> Result<Option<Resolved<'a>>> {
    if candidate.unresolved {
        stats.candidates_unresolved = stats.candidates_unresolved.saturating_add(1);
        return Ok(None);
    }
    let identity = stored_identity(candidate)?;
    let in_domain = domains.contains(&identity.domain());
    if !in_domain {
        stats.candidates_filtered_by_domain = stats.candidates_filtered_by_domain.saturating_add(1);
    }
    Ok(Some(Resolved {
        identity,
        in_domain,
        row: candidate,
    }))
}

/// Every verdict that matched a candidate its own observation returned,
/// counting the unresolved, stale and recall-miss refusals on the way past.
pub fn verdicts(
    input: &RowInput<'_>,
    index: &BTreeMap<String, usize>,
    resolved: &[Vec<Resolved<'_>>],
    stats: &mut RowStats,
) -> Result<Vec<Verdict>> {
    let unresolved = unresolved_refs(input.snapshot, index);
    // The identity list per observation is built ONCE. Rebuilding it inside the
    // judgment loop would clone every candidate's identity per verdict, which
    // is quadratic in a pool the capture bound already allows to be large.
    let identities: Vec<Vec<CandidateIdentity>> = resolved
        .iter()
        .map(|bucket| bucket.iter().map(|r| r.identity.clone()).collect())
        .collect();
    let mut accepted = Vec::new();
    for (slot, judgment) in admitted(&input.snapshot.judgments, index, |j| &j.observation_id) {
        stats.judgments_scanned = stats.judgments_scanned.saturating_add(1);
        if unresolved.contains(&(slot, judgment.candidate_ref.as_str())) {
            stats.judgments_on_unresolved_candidates =
                stats.judgments_on_unresolved_candidates.saturating_add(1);
            continue;
        }
        let Some(class) = LabelClass::parse(&judgment.provenance) else {
            continue;
        };
        let Some(bucket) = resolved.get(slot) else {
            continue;
        };
        if !input.provenance.admits(class) {
            continue;
        }
        let identity = parse_ref(&judgment.candidate_ref).map_err(|e| {
            Error::Other(format!(
                "observation `{}` holds an unreadable judgment reference: {e}",
                judgment.observation_id
            ))
        })?;
        let observed = identities.get(slot).map_or(&[][..], Vec::as_slice);
        match classify(&identity, observed, &judgment.observation_id)? {
            VerdictOutcome::Matched(candidate) => {
                accepted.extend(matched(judgment, slot, candidate, bucket, class));
            }
            VerdictOutcome::Stale => {
                stats.judgments_stale = stats.judgments_stale.saturating_add(1);
            }
            VerdictOutcome::RecallMiss => {
                stats.judgments_recall_miss = stats.judgments_recall_miss.saturating_add(1);
            }
        }
    }
    Ok(accepted)
}

/// The admitted verdict for a matched candidate, or `None` when this run's
/// domain filter excluded the candidate it matched — a filter, not a defect,
/// so it is silent rather than counted as a refusal.
fn matched(
    judgment: &DatasetJudgment,
    observation: usize,
    candidate: usize,
    bucket: &[Resolved<'_>],
    class: LabelClass,
) -> Option<Verdict> {
    let row = bucket.get(candidate).filter(|r| r.in_domain)?;
    Some(Verdict {
        observation,
        candidate,
        candidate_ref: row.row.candidate_ref.clone(),
        label: RecordLabel {
            relevance: judgment.relevance.clamp(0, i64::from(MAX_RELEVANCE)) as u8,
            provenance: judgment.provenance.clone(),
            at: judgment.at.clone(),
        },
        class,
    })
}

/// The `(observation, candidate)` pairs a contradiction resolution suppresses.
pub fn contradictions(
    contexts: &[ObservationContext],
    verdicts: &[&Verdict],
    stats: &mut RowStats,
) -> BTreeSet<(usize, usize)> {
    let keys: Vec<String> = verdicts
        .iter()
        .map(|v| {
            let group = contexts
                .get(v.observation)
                .map_or("", |c| c.query_group.as_str());
            format!("{group}\u{0}{}\u{0}{}", v.candidate_ref, v.label.provenance)
        })
        .collect();
    let inputs: Vec<ContradictionInput<'_>> = verdicts
        .iter()
        .zip(&keys)
        .map(|(v, key)| ContradictionInput {
            key,
            relevance: v.label.relevance,
            at: &v.label.at,
            observation_id: contexts
                .get(v.observation)
                .map_or("", |c| c.observation_id.as_str()),
        })
        .collect();
    let dropped = resolve_contradictions(&inputs);
    stats.judgments_contradictory_dropped = dropped.len() as u64;
    dropped
        .into_iter()
        .filter_map(|i| verdicts.get(i).map(|v| (v.observation, v.candidate)))
        .collect()
}

/// Build one row per surviving candidate, labelled where a verdict survived.
pub fn emit(
    resolved: &[Vec<Resolved<'_>>],
    verdicts: &[&Verdict],
    dropped: &BTreeSet<usize>,
    suppressed: &BTreeSet<(usize, usize)>,
    include_unjudged: bool,
    stats: &mut RowStats,
) -> Vec<PendingRow> {
    let labels: BTreeMap<(usize, usize), &Verdict> = verdicts
        .iter()
        .map(|v| ((v.observation, v.candidate), *v))
        .collect();
    let mut rows = Vec::new();
    for (slot, bucket) in resolved.iter().enumerate() {
        if dropped.contains(&slot) {
            continue;
        }
        for (position, candidate) in bucket.iter().enumerate() {
            if !candidate.in_domain || suppressed.contains(&(slot, position)) {
                continue;
            }
            let verdict = labels.get(&(slot, position)).copied();
            if verdict.is_none() {
                stats.candidates_unjudged = stats.candidates_unjudged.saturating_add(1);
                if !include_unjudged {
                    continue;
                }
            }
            rows.push(row_of(slot, candidate, verdict));
        }
    }
    rows
}

/// One pending row.
fn row_of(observation: usize, candidate: &Resolved<'_>, verdict: Option<&Verdict>) -> PendingRow {
    let row = candidate.row;
    PendingRow {
        observation,
        content_group: content_group(&candidate.identity),
        repo: match &candidate.identity {
            CandidateIdentity::Code(c) => Some(c.repo.clone()),
            _ => None,
        },
        identity: candidate.identity.clone(),
        candidate_ref: row.candidate_ref.clone(),
        content_version: row.content_version.clone(),
        text: row.text.clone(),
        text_sha256: row.text_sha256.clone(),
        text_full_bytes: row.text_full_bytes,
        text_truncated: row.text_truncated,
        pool_position: row.pool_position,
        returned_position: row.returned_position,
        retrieval_score: row.retrieval_score,
        rank_in_domain: row.rank_in_domain,
        tier: row.tier,
        label: verdict.map(|v| v.label.clone()),
        label_class: verdict.map(|v| v.class),
    }
}

/// Every `(observation slot, candidate_ref)` an admitted observation recorded
/// as unresolved, so a verdict naming one is reported rather than matched.
fn unresolved_refs<'a>(
    snapshot: &'a DatasetSnapshot,
    index: &BTreeMap<String, usize>,
) -> BTreeSet<(usize, &'a str)> {
    snapshot
        .candidates
        .iter()
        .filter(|c| c.unresolved)
        .filter_map(|c| {
            index
                .get(&c.observation_id)
                .map(|slot| (*slot, c.candidate_ref.as_str()))
        })
        .collect()
}

/// The identity of one stored candidate, or an error naming where it sits.
fn stored_identity(candidate: &DatasetCandidate) -> Result<CandidateIdentity> {
    parse_ref(&candidate.candidate_ref).map_err(|e| {
        Error::Other(format!(
            "observation `{}` holds an unreadable candidate at pool position {}: {e}. That row \
             was not written by this build's observation contract.",
            candidate.observation_id, candidate.pool_position
        ))
    })
}
