//! Turning a captured snapshot into exportable rows (#210): the provenance
//! filter, the contract-version refusal, and the per-observation context every
//! row of one observation shares.
//!
//! The order the pipeline runs in is part of the contract and lives in
//! [`build`]. The selection rules it drives are
//! [`super::dataset_select`]; the collapse rules are
//! [`super::dataset_dedup`].

use std::collections::{BTreeMap, BTreeSet};

use crate::domains::learning::evaluation::candidate_identity::CandidateDomain;
use crate::domains::learning::evaluation::candidate_observation::{
    EffectiveFilters, OBSERVATION_VERSION, RetrievalVersion,
};
use crate::domains::learning::evaluation::dataset_dedup::{DuplicateInput, collapse_duplicates};
use crate::domains::learning::evaluation::dataset_manifest::RowStats;
use crate::domains::learning::evaluation::dataset_record::{LabelClass, RetrievalRevision};
use crate::domains::learning::evaluation::dataset_select::{
    PendingRow, Verdict, contradictions, emit, resolve, verdicts,
};
use crate::domains::learning::evaluation::dataset_split::query_group;
use crate::prelude::*;
use crate::store::candidate_dataset::{DatasetObservation, DatasetSnapshot};

/// Which judgment provenance reaches the dataset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProvenanceFilter {
    /// Reviewed verdicts only. The default, because an implicit signal is not
    /// evaluation truth.
    Manual,
    /// Implicit signals only.
    Implicit,
    /// Both, written to separate files and never mixed.
    All,
}

impl ProvenanceFilter {
    /// The accepted words, in the order the help text lists them.
    pub const WORDS: [&'static str; 3] = ["manual", "implicit", "all"];

    /// Parse one of [`Self::WORDS`], or refuse it naming what is accepted.
    pub fn parse(word: &str) -> Result<ProvenanceFilter> {
        match word {
            "manual" => Ok(ProvenanceFilter::Manual),
            "implicit" => Ok(ProvenanceFilter::Implicit),
            "all" => Ok(ProvenanceFilter::All),
            other => Err(Error::Config(format!(
                "unknown provenance `{other}`: expected one of {}",
                Self::WORDS.join(", ")
            ))),
        }
    }

    /// The word this filter reports itself as.
    pub fn as_str(self) -> &'static str {
        match self {
            ProvenanceFilter::Manual => "manual",
            ProvenanceFilter::Implicit => "implicit",
            ProvenanceFilter::All => "all",
        }
    }

    /// Whether a verdict of `class` reaches the dataset under this filter.
    pub fn admits(self, class: LabelClass) -> bool {
        match self {
            ProvenanceFilter::All => true,
            ProvenanceFilter::Manual => class == LabelClass::Manual,
            ProvenanceFilter::Implicit => class == LabelClass::Implicit,
        }
    }

    /// Which label classes this filter writes files for.
    pub fn classes(self) -> Vec<LabelClass> {
        [LabelClass::Manual, LabelClass::Implicit]
            .into_iter()
            .filter(|c| self.admits(*c))
            .collect()
    }
}

/// Everything one observation's rows share, parsed once.
pub struct ObservationContext {
    /// `o-<yyyymmdd>-<8hex>`.
    pub observation_id: String,
    /// The `retrieval_log.query_id` of the same run, when one was written.
    pub query_id: Option<String>,
    /// The query text, verbatim.
    pub query: String,
    /// The near-duplicate key of `query`.
    pub query_group: String,
    /// Which surface produced the run.
    pub source: String,
    /// The run's start instant: the contract's `reference_time`.
    pub at: String,
    /// Whether ACT-R activation was time-independent for the run.
    pub decay_frozen: bool,
    /// Whether the capture bound cut the pool.
    pub truncated: bool,
    /// Every filter the run applied.
    pub filters: EffectiveFilters,
    /// The full ranking and corpus version, for the manifest.
    pub retrieval: RetrievalVersion,
    /// The four-field summary a record carries.
    pub revision: RetrievalRevision,
    /// The key two duplicate observations share.
    pub dedup_key: String,
}

/// What one build reads.
pub struct RowInput<'a> {
    /// The three tables, read as one consistent view.
    pub snapshot: &'a DatasetSnapshot,
    /// Which judgment provenance reaches the dataset.
    pub provenance: ProvenanceFilter,
    /// Which domains reach the dataset.
    pub domains: &'a [CandidateDomain],
    /// Whether a retrieved candidate with no verdict is emitted unlabelled.
    pub include_unjudged: bool,
}

/// What one build produced.
pub struct BuiltRows {
    /// The admitted observations, in capture order.
    pub contexts: Vec<ObservationContext>,
    /// Every exportable row.
    pub rows: Vec<PendingRow>,
    /// What was refused, dropped and counted along the way.
    pub stats: RowStats,
}

/// Apply the pipeline to `input` and return every exportable row.
///
/// The order is fixed: refuse an unknown contract version, drop unresolved
/// candidates and out-of-domain ones, collapse duplicate observations,
/// classify each verdict against what its own observation returned, resolve
/// contradictions, and only then build rows. The two drops precede the
/// collapse so a verdict that could never be exported cannot decide which of
/// two duplicate observations survives.
pub fn build(input: RowInput<'_>) -> Result<BuiltRows> {
    let mut stats = RowStats {
        observations_scanned: input.snapshot.observations.len() as u64,
        ..RowStats::default()
    };
    let (contexts, index) = contexts(&input.snapshot.observations, &mut stats)?;
    let resolved = resolve(input.snapshot, &index, input.domains, &mut stats)?;
    let judged = verdicts(&input, &index, &resolved, &mut stats)?;

    let dropped = duplicates(&contexts, &judged, &mut stats);
    let live: Vec<&Verdict> = judged
        .iter()
        .filter(|v| !dropped.contains(&v.observation))
        .collect();
    let suppressed = contradictions(&contexts, &live, &mut stats);
    let rows = emit(
        &resolved,
        &live,
        &dropped,
        &suppressed,
        input.include_unjudged,
        &mut stats,
    );
    Ok(BuiltRows {
        contexts,
        rows,
        stats,
    })
}

/// The observation slots the duplicate collapse removes.
pub fn duplicates(
    contexts: &[ObservationContext],
    verdicts: &[Verdict],
    stats: &mut RowStats,
) -> BTreeSet<usize> {
    let mut counts: BTreeMap<usize, u64> = BTreeMap::new();
    for verdict in verdicts {
        let slot = counts.entry(verdict.observation).or_default();
        *slot = slot.saturating_add(1);
    }
    let inputs: Vec<DuplicateInput<'_>> = contexts
        .iter()
        .enumerate()
        .map(|(slot, ctx)| DuplicateInput {
            dedup_key: &ctx.dedup_key,
            at: &ctx.at,
            observation_id: &ctx.observation_id,
            verdicts: counts.get(&slot).copied().unwrap_or(0),
        })
        .collect();
    let dropped = collapse_duplicates(&inputs);
    stats.duplicate_observations_dropped = dropped.len() as u64;
    dropped
}

/// Admit every observation this build can read, refusing an unknown contract
/// version by name rather than guessing at its meaning.
fn contexts(
    observations: &[DatasetObservation],
    stats: &mut RowStats,
) -> Result<(Vec<ObservationContext>, BTreeMap<String, usize>)> {
    let mut contexts = Vec::with_capacity(observations.len());
    let mut index = BTreeMap::new();
    for observation in observations {
        if observation.observation_version != i64::from(OBSERVATION_VERSION) {
            stats.observations_refused_version =
                stats.observations_refused_version.saturating_add(1);
            tracing::warn!(
                observation = %observation.observation_id,
                found = observation.observation_version,
                expected = OBSERVATION_VERSION,
                "refusing an observation written at an unknown contract version"
            );
            continue;
        }
        if observation.truncated {
            stats.observations_truncated = stats.observations_truncated.saturating_add(1);
        }
        index.insert(observation.observation_id.clone(), contexts.len());
        contexts.push(context_of(observation)?);
    }
    Ok((contexts, index))
}

/// One observation's parsed context. A `filters_json` or `retrieval_json` this
/// build cannot read fails the export: both are contract fields a record
/// carries verbatim, so neither may be dropped to keep going.
fn context_of(observation: &DatasetObservation) -> Result<ObservationContext> {
    let filters: EffectiveFilters = column(&observation.filters_json, observation, "filters_json")?;
    let retrieval: RetrievalVersion =
        column(&observation.retrieval_json, observation, "retrieval_json")?;
    Ok(ObservationContext {
        observation_id: observation.observation_id.clone(),
        query_id: observation.query_id.clone(),
        query_group: query_group(&observation.query),
        source: observation.source.clone(),
        at: observation.at.clone(),
        decay_frozen: observation.decay_frozen,
        truncated: observation.truncated,
        revision: RetrievalRevision {
            binary_version: retrieval.binary_version.clone(),
            schema_version: retrieval.schema_version.clone(),
            knobs_hash: observation.knobs_hash.clone(),
            corpus_digest: observation.corpus_digest.clone(),
        },
        dedup_key: format!(
            "{}\u{0}{}\u{0}{}\u{0}{}",
            observation.query,
            observation.knobs_hash,
            observation.corpus_digest,
            observation.filters_json
        ),
        query: observation.query.clone(),
        filters,
        retrieval,
    })
}

/// One JSON column, or an error naming the observation and the column.
fn column<T: serde::de::DeserializeOwned>(
    body: &str,
    observation: &DatasetObservation,
    name: &str,
) -> Result<T> {
    serde_json::from_str(body).map_err(|e| {
        Error::Other(format!(
            "observation `{}` holds a `{name}` this build cannot read: {e}",
            observation.observation_id
        ))
    })
}

#[cfg(test)]
#[path = "tests/dataset_rows.rs"]
mod tests;
