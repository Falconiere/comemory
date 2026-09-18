//! Bulk windowed reads over `candidate_query_observations`,
//! `candidate_observations` and `candidate_judgments`: the input side of the
//! reviewed dataset export (#210).
//!
//! `candidate_observations::fetch` answers "show me this one observation" for
//! `comemory judge`. An export asks a different question — "give me every
//! observation captured in this window, with every candidate and every
//! verdict" — and needs columns that read does not project, so it gets its own
//! module rather than widening a per-observation read into a bulk one.
//!
//! Two properties are structural here rather than merely intended.
//!
//! All three reads run inside ONE `unchecked_transaction`. A capture landing
//! between two separate statements would hand the caller a header with no
//! candidates, and the export's whole reproducibility claim rests on "the same
//! snapshot" meaning something.
//!
//! `locator_json` is never selected. #209 gives the locator no index precisely
//! so that "never match a judgment on a display field" is enforced rather than
//! stated; this module extends that to the training data, so a title, a screen
//! path or a recycled `code_symbols` rowid cannot reach an exported record at
//! all.

use rusqlite::{Connection, Row, params};

use crate::prelude::*;

/// One `candidate_query_observations` row, with every column the export reads.
pub struct DatasetObservation {
    /// `o-<yyyymmdd>-<8hex>`.
    pub observation_id: String,
    /// The contract version this row was written at. A reader that does not
    /// recognise it must refuse the record rather than guess at its meaning.
    pub observation_version: i64,
    /// The `retrieval_log.query_id` of the same run, when one was written.
    pub query_id: Option<String>,
    /// The query text, verbatim as retrieval received it.
    pub query: String,
    /// Which surface produced the run (a `utilities::telemetry::source` const).
    pub source: String,
    /// `EffectiveFilters` as JSON.
    pub filters_json: String,
    /// `RetrievalVersion` as JSON.
    pub retrieval_json: String,
    /// `RetrievalVersion::knobs_hash`.
    pub knobs_hash: String,
    /// `RetrievalVersion::corpus.digest`.
    pub corpus_digest: String,
    /// Whether ACT-R activation was time-independent for this run.
    pub decay_frozen: bool,
    /// How many candidate rows this observation holds.
    ///
    /// `pool_size`, `page_limit` and `page_offset` are deliberately absent:
    /// the export carries none of them onto a record, and a projection that
    /// reads a column nobody consumes is dead weight on every row.
    pub candidate_count: i64,
    /// Whether the configured candidate bound cut the pool. A consumer must
    /// not compute candidate-pool recall from a truncated observation.
    pub truncated: bool,
    /// The run's start instant. This is the contract's `reference_time`.
    pub at: String,
}

/// One `candidate_observations` row. `locator_json` is deliberately absent:
/// see this module's own documentation.
pub struct DatasetCandidate {
    /// The observation this candidate belongs to.
    pub observation_id: String,
    /// 1-based position in the candidate pool, in retrieval's own order.
    pub pool_position: i64,
    /// `memory` | `code` | `document`.
    pub domain: String,
    /// The domain-qualified reference string: the only matchable key.
    pub candidate_ref: String,
    /// The content version carried inside `candidate_ref`; empty when the
    /// candidate is unresolved.
    pub content_version: String,
    /// Whether this candidate has no content snapshot and can never be judged.
    pub unresolved: bool,
    /// 1-based position on the displayed page, when it was on it.
    pub returned_position: Option<i64>,
    /// The fused score retrieval assigned.
    pub retrieval_score: f64,
    /// 1-based position within the candidate's own leg, before fusion.
    pub rank_in_domain: i64,
    /// Memory lexical-ladder tier, where the domain has one.
    pub tier: Option<i64>,
    /// The bounded passage retrieval matched on.
    pub text: String,
    /// Digest of the FULL passage, before bounding.
    pub text_sha256: String,
    /// Byte length of the full passage, before bounding.
    pub text_full_bytes: i64,
    /// Whether `text` is shorter than the full passage.
    pub text_truncated: bool,
}

/// One `candidate_judgments` row.
pub struct DatasetJudgment {
    /// The captured query this verdict answers.
    pub observation_id: String,
    /// The candidate that was judged, exactly as it was observed.
    pub candidate_ref: String,
    /// `memory` | `code` | `document`.
    pub domain: String,
    /// Graded relevance, `0..=3`. `0` is a reviewed "not relevant".
    pub relevance: i64,
    /// `manual` or `implicit`.
    pub provenance: String,
    /// RFC3339 time the verdict was recorded.
    pub at: String,
}

/// The three tables read as one consistent view of one `at` window.
pub struct DatasetSnapshot {
    /// Observations in the window, ascending by `(at, observation_id)`.
    pub observations: Vec<DatasetObservation>,
    /// Their candidates, ascending by `(observation_id, pool_position)`.
    pub candidates: Vec<DatasetCandidate>,
    /// Their judgments, ascending by `(observation_id, candidate_ref)`.
    pub judgments: Vec<DatasetJudgment>,
}

/// The three statements, spelled out rather than assembled, so the reader sees
/// exactly what each one selects. All three take the same two bound parameters
/// and apply the same half-open `at` predicate; `NULL` on either side means
/// unbounded, so one statement serves every combination of the two optional
/// bounds without building SQL by concatenation.
const OBSERVATIONS_SQL: &str = "\
    SELECT observation_id, observation_version, query_id, query, source, \
           filters_json, retrieval_json, knobs_hash, corpus_digest, decay_frozen, \
           candidate_count, truncated, at \
      FROM candidate_query_observations \
     WHERE (?1 IS NULL OR at >= ?1) AND (?2 IS NULL OR at < ?2) \
     ORDER BY at, observation_id";

/// Every candidate of those observations. No `locator_json`.
const CANDIDATES_SQL: &str = "\
    SELECT observation_id, pool_position, domain, candidate_ref, content_version, \
           unresolved, returned_position, retrieval_score, rank_in_domain, tier, \
           text, text_sha256, text_full_bytes, text_truncated \
      FROM candidate_observations \
     WHERE observation_id IN (SELECT observation_id FROM candidate_query_observations \
                               WHERE (?1 IS NULL OR at >= ?1) AND (?2 IS NULL OR at < ?2)) \
     ORDER BY observation_id, pool_position";

/// Every verdict recorded against those observations.
const JUDGMENTS_SQL: &str = "\
    SELECT observation_id, candidate_ref, domain, relevance, provenance, at \
      FROM candidate_judgments \
     WHERE observation_id IN (SELECT observation_id FROM candidate_query_observations \
                               WHERE (?1 IS NULL OR at >= ?1) AND (?2 IS NULL OR at < ?2)) \
     ORDER BY observation_id, candidate_ref";

/// Read every observation whose `at` falls in the half-open window
/// `[since, until)`, with its candidates and its judgments, as one consistent
/// view.
///
/// `since` and `until` must already be rendered in
/// [`super::memory_row::iso_format`], the fixed-width ISO-8601 UTC shape the
/// `at` column is written in, so a plain string comparison is chronological —
/// the same contract `super::gc_learning::evict_before` relies on.
///
/// The three row mappers are closures rather than named functions because each
/// is nothing but this statement's column order; naming them would invite a
/// reader to look for a difference that is not there.
pub fn snapshot(
    conn: &Connection,
    since: Option<&str>,
    until: Option<&str>,
) -> Result<DatasetSnapshot> {
    let tx = conn.unchecked_transaction()?;
    let observations = windowed(&tx, OBSERVATIONS_SQL, since, until, |r| {
        Ok(DatasetObservation {
            observation_id: r.get(0)?,
            observation_version: r.get(1)?,
            query_id: r.get(2)?,
            query: r.get(3)?,
            source: r.get(4)?,
            filters_json: r.get(5)?,
            retrieval_json: r.get(6)?,
            knobs_hash: r.get(7)?,
            corpus_digest: r.get(8)?,
            decay_frozen: r.get::<_, i64>(9)? != 0,
            candidate_count: r.get(10)?,
            truncated: r.get::<_, i64>(11)? != 0,
            at: r.get(12)?,
        })
    })?;
    let candidates = windowed(&tx, CANDIDATES_SQL, since, until, |r| {
        Ok(DatasetCandidate {
            observation_id: r.get(0)?,
            pool_position: r.get(1)?,
            domain: r.get(2)?,
            candidate_ref: r.get(3)?,
            content_version: r.get(4)?,
            unresolved: r.get::<_, i64>(5)? != 0,
            returned_position: r.get(6)?,
            retrieval_score: r.get(7)?,
            rank_in_domain: r.get(8)?,
            tier: r.get(9)?,
            text: r.get(10)?,
            text_sha256: r.get(11)?,
            text_full_bytes: r.get(12)?,
            text_truncated: r.get::<_, i64>(13)? != 0,
        })
    })?;
    let judgments = windowed(&tx, JUDGMENTS_SQL, since, until, |r| {
        Ok(DatasetJudgment {
            observation_id: r.get(0)?,
            candidate_ref: r.get(1)?,
            domain: r.get(2)?,
            relevance: r.get(3)?,
            provenance: r.get(4)?,
            at: r.get(5)?,
        })
    })?;
    tx.commit()?;
    Ok(DatasetSnapshot {
        observations,
        candidates,
        judgments,
    })
}

/// One windowed read: prepare, bind the two optional bounds, map, own.
fn windowed<T>(
    conn: &Connection,
    sql: &str,
    since: Option<&str>,
    until: Option<&str>,
    row: impl Fn(&Row<'_>) -> rusqlite::Result<T>,
) -> Result<Vec<T>> {
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map(params![since, until], row)?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

#[cfg(test)]
#[path = "tests/candidate_dataset.rs"]
mod tests;
