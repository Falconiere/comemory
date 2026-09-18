//! `candidate_query_observations` + `candidate_observations` row CRUD: the
//! persisted form of the #208 candidate observation contract (#209).
//!
//! One captured query is one header row plus one row per pooled candidate,
//! written as a single unit inside this module's own transaction, because a
//! query-time capture holds only a `&Connection`.
//!
//! Identity lives entirely in `candidate_ref`, which `parse_ref` inverts. The
//! display fields ride along as opaque `locator_json` and are never a
//! predicate here: that is how "never match on a locator" is enforced rather
//! than merely stated.

use rusqlite::Connection;

use super::{
    orm,
    schema_learning::{
        CandidateObservations, CandidateQueryObservations, candidate_observations as col,
        candidate_query_observations as head,
    },
};
use crate::prelude::*;
use toolu_orm::core::query_column::CommonOps;

/// Insert parameters for one `candidate_query_observations` row, bundled
/// rather than passed as sixteen positional arguments.
pub struct NewObservation<'a> {
    /// `o-<yyyymmdd>-<8hex>`.
    pub observation_id: &'a str,
    /// The contract version this row is written at.
    pub observation_version: i64,
    /// The `retrieval_log.query_id` of the same run, when one was written.
    pub query_id: Option<&'a str>,
    /// The query text, verbatim.
    pub query: &'a str,
    /// A `crate::utilities::telemetry::source` const.
    pub source: &'a str,
    /// `EffectiveFilters` as JSON.
    pub filters_json: &'a str,
    /// `RetrievalVersion` as JSON.
    pub retrieval_json: &'a str,
    /// `RetrievalVersion::knobs_hash`.
    pub knobs_hash: &'a str,
    /// `RetrievalVersion::corpus.digest`.
    pub corpus_digest: &'a str,
    /// Whether ACT-R activation was time-independent for this run.
    pub decay_frozen: bool,
    /// The shared pool size every leg was fetched at.
    pub pool_size: i64,
    /// Page size the `returned_position` values were cut at.
    pub page_limit: i64,
    /// Page offset the run was sliced from.
    pub page_offset: i64,
    /// Whether the configured candidate bound cut the pool.
    pub truncated: bool,
    /// The run's start instant, `super::memory_row::iso_format`.
    pub at: &'a str,
}

/// Insert parameters for one `candidate_observations` row.
pub struct NewCandidate<'a> {
    /// 1-based position in the candidate pool.
    pub pool_position: i64,
    /// `memory` | `code` | `document`.
    pub domain: &'a str,
    /// The domain-qualified reference string.
    pub candidate_ref: &'a str,
    /// The content version carried inside `candidate_ref`.
    pub content_version: &'a str,
    /// Whether this candidate has no content snapshot.
    pub unresolved: bool,
    /// 1-based position on the displayed page, when it was on it.
    pub returned_position: Option<i64>,
    /// The fused score retrieval assigned.
    pub retrieval_score: f64,
    /// 1-based position within the candidate's own leg.
    pub rank_in_domain: i64,
    /// Memory lexical-ladder tier, where the domain has one.
    pub tier: Option<i64>,
    /// The bounded passage.
    pub text: &'a str,
    /// Digest of the full passage, before bounding.
    pub text_sha256: &'a str,
    /// Byte length of the full passage, before bounding.
    pub text_full_bytes: i64,
    /// Whether `text` is shorter than the full passage.
    pub text_truncated: bool,
    /// `CandidateLocator` as JSON. Display only.
    pub locator_json: &'a str,
}

/// One stored header row, projected to what a reader of a captured
/// observation needs. The table keeps every contract field; a consumer that
/// needs more (the #210 export) projects its own columns.
pub struct StoredObservation {
    /// `o-<yyyymmdd>-<8hex>`.
    pub observation_id: String,
    /// The contract version the row was written at. A reader that does not
    /// recognise it must refuse the record rather than guess.
    pub observation_version: i64,
    /// The query text, verbatim.
    pub query: String,
    /// The ranking configuration digest.
    pub knobs_hash: String,
    /// The corpus snapshot digest.
    pub corpus_digest: String,
    /// How many candidate rows this observation has.
    pub candidate_count: i64,
    /// Whether the candidate bound cut the pool. A consumer must not compute
    /// candidate-pool recall from a truncated observation.
    pub truncated: bool,
}

/// One stored candidate row, projected to what resolving and reporting a
/// verdict needs.
pub struct StoredCandidate {
    /// 1-based position in the candidate pool.
    pub pool_position: i64,
    /// `memory` | `code` | `document`.
    pub domain: String,
    /// The domain-qualified reference string; `parse_ref` inverts it into the
    /// full identity, content version included.
    pub candidate_ref: String,
    /// Whether this candidate has no content snapshot and can never be judged.
    pub unresolved: bool,
    /// 1-based position on the displayed page, when it was on it.
    pub returned_position: Option<i64>,
    /// `CandidateLocator` as JSON. Display only.
    pub locator_json: String,
}

/// Write one captured query: the header plus every candidate, as one unit.
///
/// The transaction is opened here rather than by the caller because the caller
/// is a query-time capture holding a shared `&Connection`. A failure rolls the
/// whole observation back, so a header never survives without its candidates —
/// a half-written pool would misreport candidate-pool recall, which is the one
/// number this table exists to make measurable.
pub fn insert(
    conn: &Connection,
    header: &NewObservation<'_>,
    candidates: &[NewCandidate<'_>],
) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    orm::execute(
        &tx,
        CandidateQueryObservations::insert()
            .set(&head::observation_id, header.observation_id)
            .set(&head::observation_version, header.observation_version)
            .set(&head::query_id, header.query_id)
            .set(&head::query, header.query)
            .set(&head::source, header.source)
            .set(&head::filters_json, header.filters_json)
            .set(&head::retrieval_json, header.retrieval_json)
            .set(&head::knobs_hash, header.knobs_hash)
            .set(&head::corpus_digest, header.corpus_digest)
            .set(&head::decay_frozen, i64::from(header.decay_frozen))
            .set(&head::pool_size, header.pool_size)
            .set(&head::page_limit, header.page_limit)
            .set(&head::page_offset, header.page_offset)
            .set(&head::candidate_count, candidates.len() as i64)
            .set(&head::truncated, i64::from(header.truncated))
            .set(&head::at, header.at)
            .to_sql(),
    )?;
    for candidate in candidates {
        orm::execute(&tx, candidate_sql(header.observation_id, candidate))?;
    }
    tx.commit()?;
    Ok(())
}

/// One candidate row's generated INSERT. Split out so [`insert`] stays inside
/// the function-size ceiling and the column list has one home.
fn candidate_sql(
    observation_id: &str,
    row: &NewCandidate<'_>,
) -> (String, Vec<toolu_orm::core::value::Value>) {
    CandidateObservations::insert()
        .set(&col::observation_id, observation_id)
        .set(&col::pool_position, row.pool_position)
        .set(&col::domain, row.domain)
        .set(&col::candidate_ref, row.candidate_ref)
        .set(&col::content_version, row.content_version)
        .set(&col::unresolved, i64::from(row.unresolved))
        .set(&col::returned_position, row.returned_position)
        .set(&col::retrieval_score, row.retrieval_score)
        .set(&col::rank_in_domain, row.rank_in_domain)
        .set(&col::tier, row.tier)
        .set(&col::text, row.text)
        .set(&col::text_sha256, row.text_sha256)
        .set(&col::text_full_bytes, row.text_full_bytes)
        .set(&col::text_truncated, i64::from(row.text_truncated))
        .set(&col::locator_json, row.locator_json)
        .to_sql()
}

/// One captured observation: its header and every candidate, ascending by
/// `pool_position` — the order retrieval produced them in.
///
/// `None` when no such observation exists (never captured, or already evicted
/// by the retention sweep). Both reads are one function because the only
/// consumer needs both, and because two separately-written projections of the
/// same observation are a near-duplicate this module would otherwise carry.
pub fn fetch(
    conn: &Connection,
    observation_id: &str,
) -> Result<Option<(StoredObservation, Vec<StoredCandidate>)>> {
    let header = orm::query_optional(
        conn,
        CandidateQueryObservations::select()
            .columns_typed(&[
                &head::observation_id,
                &head::observation_version,
                &head::query,
                &head::knobs_hash,
                &head::corpus_digest,
                &head::candidate_count,
                &head::truncated,
            ])
            .filter(head::observation_id.eq(observation_id))
            .to_sql(),
        |r| {
            Ok(StoredObservation {
                observation_id: r.get(0)?,
                observation_version: r.get(1)?,
                query: r.get(2)?,
                knobs_hash: r.get(3)?,
                corpus_digest: r.get(4)?,
                candidate_count: r.get(5)?,
                truncated: r.get::<_, i64>(6)? != 0,
            })
        },
    )?;
    let Some(header) = header else {
        return Ok(None);
    };
    let candidates = orm::query_all(
        conn,
        CandidateObservations::select()
            .columns_typed(&[
                &col::pool_position,
                &col::domain,
                &col::candidate_ref,
                &col::unresolved,
                &col::returned_position,
                &col::locator_json,
            ])
            .filter(col::observation_id.eq(observation_id))
            .order_by(col::pool_position.asc())
            .to_sql(),
        |r| {
            Ok(StoredCandidate {
                pool_position: r.get(0)?,
                domain: r.get(1)?,
                candidate_ref: r.get(2)?,
                unresolved: r.get::<_, i64>(3)? != 0,
                returned_position: r.get(4)?,
                locator_json: r.get(5)?,
            })
        },
    )?;
    Ok(Some((header, candidates)))
}

/// Redact every captured passage belonging to the purged memory `memory_id`:
/// blank the text, zero its size, and mark the candidate unresolved.
///
/// The row and its pool position stay. Deleting it would corrupt a recorded
/// pool's recall; keeping the body would resurrect deleted content.
///
/// A memory reference is `memory:<id>:<content_hash>`, so the rows are those
/// prefixed `memory:<id>:`. `substr` rather than `LIKE` keeps the comparison
/// literal by construction instead of by escaping.
pub fn redact_memory(conn: &Connection, memory_id: &str) -> Result<u64> {
    let prefix = format!("memory:{memory_id}:");
    let changed = conn.execute(
        "UPDATE candidate_observations \
            SET text = '', text_full_bytes = 0, text_truncated = 0, unresolved = 1 \
          WHERE domain = 'memory' AND substr(candidate_ref, 1, length(?1)) = ?1",
        [&prefix],
    )?;
    Ok(changed as u64)
}

/// Evict every observation older than `cutoff` that carries no judgment, with
/// its candidate rows. Returns `(headers deleted, candidate rows deleted)`.
///
/// A judged observation is retained however old it is: the reviewed verdict is
/// the expensive artifact, and evicting the observation would leave a verdict
/// with no passage and no content version to verify it against.
///
/// `cutoff` must already be rendered in `super::memory_row::iso_format`, the
/// fixed-width ISO-8601 UTC shape the `at` column is written in, so the plain
/// string `<` compares chronologically — the same contract
/// `super::gc_learning::evict_before` relies on.
pub fn evict_unjudged_before(conn: &Connection, cutoff: &str) -> Result<(u64, u64)> {
    let candidates = conn.execute(
        "DELETE FROM candidate_observations WHERE observation_id IN (\
             SELECT observation_id FROM candidate_query_observations \
              WHERE at < ?1 \
                AND observation_id NOT IN (SELECT observation_id FROM candidate_judgments))",
        [cutoff],
    )?;
    let headers = conn.execute(
        "DELETE FROM candidate_query_observations \
          WHERE at < ?1 \
            AND observation_id NOT IN (SELECT observation_id FROM candidate_judgments)",
        [cutoff],
    )?;
    Ok((headers as u64, candidates as u64))
}

#[cfg(test)]
#[path = "tests/candidate_observations.rs"]
mod tests;
