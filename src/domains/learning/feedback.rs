//! `domains::learning::feedback::{Request, Response, run}` — the shared middle of
//! `comemory feedback` / `POST /api/v1/feedback` (and, through
//! `serve::routes::search::into_feedback`, of
//! `POST /api/v1/search/{query_id}/feedback`): validate the query id
//! shape, validate the four id lists and the optional `source`, and record
//! both memory and code verdicts under one provenance. Moved out of
//! `cli::feedback::run` (Binding Rule 1).
//!
//! Opens its own `StatsDb` (the same file as `Ctx::conn`) and reserves one
//! immediate transaction for the known-query read plus every memory and code
//! verdict. A mixed request therefore commits once or rolls back as a unit.

use serde::{Deserialize, Serialize};

use crate::domains::learning::code_feedback::write_code_with_provenance;
use crate::domains::learning::feedback_tracking::{Source, write_with_provenance};
use crate::domains::learning::telemetry::StatsDb;
use crate::prelude::*;
use crate::store::connection::write_transaction;
use crate::store::retrieval_log;
use crate::utilities::context::Ctx;
use crate::utilities::id_list::{parse_id_csv, parse_symbol_id_csv};
use crate::utilities::query_id::is_valid_query_id;

/// `comemory feedback` / `POST /api/v1/feedback` request.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Id of the originating search query (`q-<yyyymmdd>-<8hex>`).
    pub query_id: String,
    /// Memory ids that were used.
    #[serde(default)]
    pub used: Vec<String>,
    /// Memory ids that were judged irrelevant.
    #[serde(default)]
    pub irrelevant: Vec<String>,
    /// Code-symbol ids (positive integers as strings) that were used.
    #[serde(default)]
    pub used_code: Vec<String>,
    /// Code-symbol ids that were judged irrelevant.
    #[serde(default)]
    pub irrelevant_code: Vec<String>,
    /// `explicit` (the default when omitted or `null`) | `implicit` — the
    /// provenance every verdict in this call is stored under (`manual` /
    /// `implicit`, see [`Source`]). Anything else is `Error::BadRequest`
    /// (HTTP `400 bad_request`), raised before the database is opened.
    /// HTTP-only in practice: `comemory feedback` never sets it, a typed
    /// verdict being a human one by definition (#130).
    #[serde(default)]
    pub source: Option<String>,
}

/// `comemory feedback` / `POST /api/v1/feedback` response — the same fields
/// the CLI's `--json` output already emits (minus its ad-hoc `ok`, which the
/// HTTP envelope's own `ok` already covers).
#[derive(Serialize, Debug)]
pub struct Response {
    /// Number of memory ids recorded as used.
    pub used: usize,
    /// Number of memory ids recorded as irrelevant.
    pub irrelevant: usize,
    /// Number of code-symbol ids recorded as used.
    pub used_code: usize,
    /// Number of code-symbol ids recorded as irrelevant.
    pub irrelevant_code: usize,
    /// The query id feedback was recorded against.
    pub query_id: String,
    /// Whether `query_id` was found in `retrieval_log`. `false` only warns —
    /// the verdicts are still recorded.
    pub known_query: bool,
    /// The `feedback_events.provenance` every verdict in this call was
    /// stored under: `manual` or `implicit`. Echoed so an HTTP caller can
    /// confirm what landed without opening the database; the CLI's `--json`
    /// ack deliberately does not print it (`cli::feedback::emit` keeps its
    /// pre-extraction byte shape).
    pub provenance: String,
}

/// Record feedback for each id provided in one immediate transaction after
/// validating every field. Memory and code verdicts share the same commit so
/// a failed code identity cannot leave memory counters or events behind.
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Response> {
    if !is_valid_query_id(&req.query_id) {
        // `Error::Config` matches `cli::feedback::run`'s original check
        // exactly — same exit code (78, AC-13); over HTTP both map to 400.
        return Err(Error::Config(format!(
            "invalid query id `{}` (expected q-<yyyymmdd>-<8hex>, as printed by comemory search)",
            req.query_id
        )));
    }
    // Validate ALL FOUR lists before the db opens so a bad id in any field
    // cannot leave another field's half already committed. Flag names here
    // are the CLI `--flag` spellings (not the JSON field names) so the error
    // text matches `cli::feedback::run`'s original byte-for-byte (AC-13);
    // the same message reaches HTTP callers — a known, accepted rough edge.
    let used_ids = parse_id_csv(&req.used.join(","), "--used")?;
    let irrelevant_ids = parse_id_csv(&req.irrelevant.join(","), "--irrelevant")?;
    let used_code_ids = parse_symbol_id_csv(&req.used_code.join(","), "--used-code")?;
    let irrelevant_code_ids =
        parse_symbol_id_csv(&req.irrelevant_code.join(","), "--irrelevant-code")?;
    // `source` is checked after the query-id and id-list shape checks, so
    // their errors keep the precedence (and exit code) they had before the
    // field existed, and before the db opens — a bad body creates no
    // database (AC-6).
    let provenance = req
        .source
        .as_deref()
        .map_or(Ok(Source::default()), Source::parse)?
        .provenance();

    let mut db = StatsDb::open(ctx.paths.stats_db())?;
    let tx = write_transaction(db.conn_mut())?;
    let known = retrieval_log::contains_query_id(&tx, &req.query_id)?;
    if !known {
        tracing::warn!(query_id = %req.query_id,
            "query id not found in retrieval_log (evicted or never logged); recording anyway");
    }
    write_with_provenance(&tx, &req.query_id, &used_ids, &irrelevant_ids, provenance)?;
    write_code_with_provenance(
        &tx,
        &req.query_id,
        &used_code_ids,
        &irrelevant_code_ids,
        provenance,
    )?;
    tx.commit()?;

    Ok(Response {
        used: used_ids.len(),
        irrelevant: irrelevant_ids.len(),
        used_code: used_code_ids.len(),
        irrelevant_code: irrelevant_code_ids.len(),
        query_id: req.query_id,
        known_query: known,
        provenance: provenance.to_string(),
    })
}

#[cfg(test)]
#[path = "tests/feedback.rs"]
mod tests;
