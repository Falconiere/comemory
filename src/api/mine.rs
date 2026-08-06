//! `api::mine::{Request, run}` — the shared middle of `comemory mine` /
//! `POST /api/v1/mine`: distill (failed_term → fix_term) expansion mappings
//! from reformulation pairs in `retrieval_log`, and optionally rebuild the
//! `query_expansions` table the tier-4 lexical ladder reads. Moved out of
//! `cli::mine::run` (Binding Rule 1).
//!
//! Always touches the DB (a scan at minimum) — no conn-free special case.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::api::Ctx;
use crate::eval::mine::{self, MinedMapping};
use crate::prelude::*;
use crate::store::memory_row;

/// `comemory mine` / `POST /api/v1/mine` request.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Rebuild the `query_expansions` table from the mined mappings
    /// (default: report only).
    #[serde(default)]
    pub apply: bool,
}

/// Distilled mappings plus whether they were applied.
#[derive(Serialize, Debug)]
pub struct Response {
    /// Every mined (term → expansion) mapping with its observation count.
    pub mappings: Vec<MinedMapping>,
    /// Whether `req.apply` rebuilt `query_expansions` from `mappings`.
    pub applied: bool,
}

/// Scan `retrieval_log` for reformulation pairs, distill term mappings, and
/// rebuild `query_expansions` from them when `req.apply` is set.
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Response> {
    let conn = ctx.conn()?;
    let mappings = mine::mine(conn)?;
    if req.apply {
        let now_iso = memory_row::iso_format(OffsetDateTime::now_utc())?;
        mine::apply(conn, &mappings, &now_iso)?;
    }
    Ok(Response {
        mappings,
        applied: req.apply,
    })
}
