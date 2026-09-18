//! `domains::learning::judge::{Request, Outcome, run}` — the shared middle of
//! `comemory judge`: typed relevance verdicts resolved against a captured
//! candidate observation (#209).
//!
//! A verdict names a candidate by its domain-qualified reference, so a target
//! that observation never produced cannot be recorded: the contract's "never
//! insert a positive retrieval did not return" rule, enforced structurally.
//! Matching runs through `evaluation::judgment`, so a content version that no
//! longer holds is refused as stale and no locator is ever consulted.
//!
//! Recording is all-or-nothing: one unresolvable reference refuses the call.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::domains::learning::evaluation::candidate_identity::{CandidateIdentity, parse_ref};
use crate::domains::learning::evaluation::candidate_observation::{
    CandidateLocator, OBSERVATION_VERSION,
};
use crate::domains::learning::evaluation::judgment::{JudgmentTarget, MAX_RELEVANCE, MatchOutcome};
use crate::domains::learning::feedback_tracking::Source;
use crate::domains::learning::observation_capture::OBSERVATION_ID_PREFIX;
use crate::prelude::*;
use crate::store::candidate_judgments::{NewJudgment, fetch_for_observation, upsert_all};
use crate::store::candidate_observations::{StoredCandidate, StoredObservation, fetch};
use crate::store::{Connection, memory_row};
use crate::utilities::context::Ctx;
use crate::utilities::dated_id::is_valid_dated_id;

/// `comemory judge` request.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// The `o-<yyyymmdd>-<8hex>` id `comemory find` printed.
    pub observation: String,
    /// `<candidate_ref>=<relevance>` verdicts. Empty reports the observation.
    #[serde(default)]
    pub refs: Vec<String>,
    /// `explicit` (the default when omitted or `null`) | `implicit` — the
    /// provenance every verdict in this call is stored under, through the same
    /// [`Source`] vocabulary `learning::feedback` uses (#130). `comemory judge`
    /// never sets it: a typed verdict is a human one by definition, so the CLI
    /// has no `--source` flag, exactly as `comemory feedback` has none. The
    /// field is the seam an implicit writer records through.
    #[serde(default)]
    pub source: Option<String>,
}

/// What one `judge` run did.
#[derive(Debug)]
pub enum Outcome {
    /// Verdicts were written.
    Recorded(Recorded),
    /// No verdict was given, so the observation was reported instead.
    Report(Box<Report>),
}

/// The acknowledgement of a recording run.
#[derive(Serialize, Debug)]
pub struct Recorded {
    /// The observation the verdicts were recorded against.
    pub observation_id: String,
    /// How many verdicts were written.
    pub recorded: usize,
    /// The `candidate_judgments.provenance` they were stored under.
    pub provenance: String,
}

/// One captured observation, as `judge` reports it when given no verdict.
#[derive(Serialize, Debug)]
pub struct Report {
    /// The observation's id.
    pub observation_id: String,
    /// The query text, verbatim.
    pub query: String,
    /// The contract version the observation was written at.
    pub observation_version: i64,
    /// The ranking configuration digest.
    pub knobs_hash: String,
    /// The corpus snapshot digest.
    pub corpus_digest: String,
    /// How many candidates the observation holds.
    pub candidate_count: i64,
    /// Whether the capture bound cut the pool.
    pub truncated: bool,
    /// Every candidate, ascending by pool position.
    pub candidates: Vec<ReportedCandidate>,
}

/// One candidate in a [`Report`].
#[derive(Serialize, Debug)]
pub struct ReportedCandidate {
    /// 1-based position in the candidate pool.
    pub pool_position: i64,
    /// 1-based position on the displayed page, when it was on it.
    pub returned_position: Option<i64>,
    /// `memory` | `code` | `document`.
    pub domain: String,
    /// The reference a verdict addresses this candidate by.
    pub candidate_ref: String,
    /// The captured headline. Display only — never a matching key.
    pub title: String,
    /// Whether the candidate has no content snapshot and cannot be judged.
    pub unresolved: bool,
    /// The relevance already recorded, when there is one.
    pub relevance: Option<i64>,
}

/// One resolved verdict, ready to write.
type Row = (String, &'static str, i64);

/// Record every verdict, or report the observation when none was given.
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Outcome> {
    // Shape, grades and provenance are checked before the database opens, so a
    // malformed invocation creates no database — the ordering
    // `learning::feedback::run` already keeps.
    let verdicts = parse_request(&req)?;
    let provenance = req
        .source
        .as_deref()
        .map_or(Ok(Source::default()), Source::parse)?
        .provenance();

    let conn = ctx.conn()?;
    let (header, candidates) = load(conn, &req.observation)?;
    if verdicts.is_empty() {
        let judged = fetch_for_observation(conn, &req.observation)?;
        return Ok(Outcome::Report(Box::new(report(
            header,
            &candidates,
            &judged,
        ))));
    }
    let rows = resolve_all(&req.observation, &verdicts, &candidates)?;
    let written = write_rows(conn, &req.observation, &rows, provenance)?;
    Ok(Outcome::Recorded(Recorded {
        observation_id: req.observation,
        recorded: written,
        provenance: provenance.to_string(),
    }))
}

/// Validate the observation-id shape and parse every verdict argument.
fn parse_request(req: &Request) -> Result<Vec<(CandidateIdentity, i64)>> {
    if !is_valid_dated_id(&req.observation, OBSERVATION_ID_PREFIX) {
        return Err(Error::Config(format!(
            "invalid observation id `{}` (expected o-<yyyymmdd>-<8hex>, as printed by \
             comemory find)",
            req.observation
        )));
    }
    req.refs.iter().map(|raw| parse_verdict(raw)).collect()
}

/// One `<candidate_ref>=<relevance>` argument, split on the LAST `=` so a
/// reference whose components contain one still parses.
fn parse_verdict(raw: &str) -> Result<(CandidateIdentity, i64)> {
    let (reference, grade) = raw.rsplit_once('=').ok_or_else(|| {
        Error::Config(format!(
            "invalid verdict `{raw}`: expected <candidate_ref>=<relevance>"
        ))
    })?;
    let relevance: i64 = grade.trim().parse().map_err(|_| {
        Error::Config(format!(
            "invalid relevance `{grade}` in `{raw}`: expected an integer in 0..={MAX_RELEVANCE}"
        ))
    })?;
    if !(0..=i64::from(MAX_RELEVANCE)).contains(&relevance) {
        return Err(Error::Config(format!(
            "relevance {relevance} in `{raw}` is out of range: expected 0..={MAX_RELEVANCE}"
        )));
    }
    Ok((parse_ref(reference)?, relevance))
}

/// The observation and its candidates, refusing an unknown id and an
/// observation written at a contract version this build does not read.
fn load(
    conn: &Connection,
    observation_id: &str,
) -> Result<(StoredObservation, Vec<StoredCandidate>)> {
    let (header, candidates) = fetch(conn, observation_id)?.ok_or_else(|| {
        Error::Unavailable(format!(
            "observation `{observation_id}` not found (never captured, or evicted by the \
             retention window)"
        ))
    })?;
    if header.observation_version != i64::from(OBSERVATION_VERSION) {
        return Err(Error::Config(format!(
            "observation `{observation_id}` was written at observation_version {} and this \
             build reads {OBSERVATION_VERSION}; refusing to guess at its meaning",
            header.observation_version
        )));
    }
    Ok((header, candidates))
}

/// Resolve every verdict against the observation's own candidates, or refuse
/// the whole call naming each reference that could not be resolved and why.
///
/// Nothing is written when any reference fails, including the ones that
/// matched: a partially recorded review is worse than a refused one, because
/// the refusal is visible and the partial record is not.
fn resolve_all(
    observation_id: &str,
    verdicts: &[(CandidateIdentity, i64)],
    candidates: &[StoredCandidate],
) -> Result<Vec<Row>> {
    let observed = candidates
        .iter()
        .map(|c| Ok((parse_ref(&c.candidate_ref)?, c)))
        .collect::<Result<Vec<_>>>()?;
    let mut rows = Vec::with_capacity(verdicts.len());
    let mut refusals = Vec::new();
    for (identity, relevance) in verdicts {
        match resolve_one(observation_id, identity, *relevance, &observed) {
            Ok(row) => rows.push(row),
            Err(why) => refusals.push(why),
        }
    }
    if refusals.is_empty() {
        return Ok(rows);
    }
    Err(Error::Config(format!(
        "no judgment was recorded; {} reference(s) could not be resolved against observation \
         `{observation_id}`:\n  {}",
        refusals.len(),
        refusals.join("\n  ")
    )))
}

/// One verdict against one observation's candidates: the row to write, or the
/// sentence explaining why it cannot be written.
fn resolve_one(
    observation_id: &str,
    identity: &CandidateIdentity,
    relevance: i64,
    observed: &[(CandidateIdentity, &StoredCandidate)],
) -> std::result::Result<Row, String> {
    let key = target_of(identity)
        .resolve(observation_id)
        .map_err(|e| e.to_string())?;
    let found = observed
        .iter()
        .map(|(seen, row)| (key.matches(seen), seen, *row))
        .find(|(outcome, _, _)| *outcome != MatchOutcome::No);
    match found {
        Some((MatchOutcome::Yes, _, row)) if !row.unresolved => Ok((
            row.candidate_ref.clone(),
            identity.domain().as_str(),
            relevance,
        )),
        Some((MatchOutcome::Yes, _, row)) => Err(format!(
            "`{}` names a candidate with no content snapshot (its row was already gone at \
             capture, or a purge redacted it), so there is nothing to judge",
            row.candidate_ref
        )),
        Some((MatchOutcome::Stale, seen, _)) => Err(format!(
            "`{}` is stale: the observation saw content version `{}`, not `{}`",
            identity.candidate_ref(),
            seen.content_version(),
            identity.content_version()
        )),
        _ => Err(format!(
            "`{}` is a candidate-pool recall miss: this observation never returned it, and a \
             positive retrieval did not return must not be inserted",
            identity.candidate_ref()
        )),
    }
}

/// Write the resolved verdicts under one provenance.
fn write_rows(
    conn: &Connection,
    observation_id: &str,
    rows: &[Row],
    provenance: &str,
) -> Result<usize> {
    let at = memory_row::iso_format(OffsetDateTime::now_utc())?;
    let written = upsert_all(
        conn,
        &rows
            .iter()
            .map(|(candidate_ref, domain, relevance)| NewJudgment {
                observation_id,
                candidate_ref,
                domain,
                relevance: *relevance,
                provenance,
                at: &at,
            })
            .collect::<Vec<_>>(),
    )?;
    Ok(usize::try_from(written).unwrap_or(rows.len()))
}

/// The read-only view of one observation: its pinned versions and every
/// candidate with the verdict already on it, if any.
fn report(
    header: StoredObservation,
    candidates: &[StoredCandidate],
    judged: &std::collections::HashMap<String, i64>,
) -> Report {
    Report {
        observation_id: header.observation_id,
        query: header.query,
        observation_version: header.observation_version,
        knobs_hash: header.knobs_hash,
        corpus_digest: header.corpus_digest,
        candidate_count: header.candidate_count,
        truncated: header.truncated,
        candidates: candidates
            .iter()
            .map(|c| ReportedCandidate {
                pool_position: c.pool_position,
                returned_position: c.returned_position,
                domain: c.domain.clone(),
                title: title_of(&c.locator_json),
                unresolved: c.unresolved,
                relevance: judged.get(&c.candidate_ref).copied(),
                candidate_ref: c.candidate_ref.clone(),
            })
            .collect(),
    }
}

/// The captured headline, or an empty string when the locator cannot be read.
/// A locator is display-only, so an unreadable one costs a title and nothing
/// else — it must never fail a review.
fn title_of(locator_json: &str) -> String {
    serde_json::from_str::<CandidateLocator>(locator_json)
        .map(|l| l.title)
        .unwrap_or_default()
}

/// The reviewed target an observed identity addresses, pinned to the content
/// version that was observed.
///
/// The inverse of a benchmark set's hand-written target: #210 renders stored
/// judgments back into a reviewed set through this, and `judge` routes every
/// reference through it so the validation is #208's own rather than a second
/// copy. A document's `document_id` is deliberately dropped — the contract
/// keys a document judgment on its source-root-relative path.
pub fn target_of(identity: &CandidateIdentity) -> JudgmentTarget {
    let mut target = JudgmentTarget {
        domain: identity.domain(),
        id: None,
        repo: None,
        path: None,
        symbol: None,
        content_hash: None,
        blob_oid: None,
        revision_hash: None,
        chunk_ordinal: None,
    };
    match identity {
        CandidateIdentity::Memory(m) => {
            target.id = Some(m.memory_id.clone());
            target.content_hash = Some(m.content_hash.clone());
        }
        CandidateIdentity::Code(c) => {
            target.repo = Some(c.repo.clone());
            target.path = Some(c.path.clone());
            target.symbol = Some(c.symbol.clone());
            target.blob_oid = Some(c.blob_oid.clone());
        }
        CandidateIdentity::Document(d) => {
            target.path = Some(d.path.clone());
            target.revision_hash = Some(d.revision_hash.clone());
            target.chunk_ordinal = Some(d.chunk_ordinal);
        }
    }
    target
}

#[cfg(test)]
#[path = "tests/judge.rs"]
mod tests;
