//! `POST /sync/import` — apply a batch of wire entries (rules 1–10).

use std::time::Instant;

use crate::domains::sync::exchange::import_rules;
use crate::domains::sync::exchange::{ImportRequest, ImportResponse, ImportStatus};
use crate::prelude::*;
use crate::store::sync_log;
use crate::utilities::activity::{self, Outcome, command};
use crate::utilities::context::Ctx;

const MAX_ENTRIES: usize = 500;

/// Apply `req.entries` in order, one transaction per entry.
pub fn run(
    ctx: &mut Ctx<'_>,
    req: ImportRequest,
    author_override: Option<&str>,
) -> Result<ImportResponse> {
    let started = Instant::now();
    let entries = req.entries.len();
    let result = apply_batch(ctx, req, author_override);
    let summary = result.as_ref().map(|r| {
        let applied = r
            .results
            .iter()
            .filter(|item| item.status == ImportStatus::Accepted)
            .count();
        serde_json::json!({
            "entries": entries,
            "applied": applied,
            "skipped": r.results.len().saturating_sub(applied),
            "head_seq": r.head_seq,
        })
    });
    let outcome = match &summary {
        Ok(value) => Outcome::Ok(value),
        Err(e) => Outcome::Failed(e),
    };
    activity::record_in(ctx, command::SYNC_IMPORT, started, &outcome, None);
    result
}

/// The import itself, wrapped by [`run`] so the activity row is written once,
/// outside the work it describes.
fn apply_batch(
    ctx: &mut Ctx<'_>,
    req: ImportRequest,
    author_override: Option<&str>,
) -> Result<ImportResponse> {
    if req.entries.len() > MAX_ENTRIES {
        return Err(Error::BadRequest(format!(
            "sync import accepts at most {MAX_ENTRIES} entries, got {}",
            req.entries.len()
        )));
    }
    let cfg = ctx.cfg;
    let mut results = Vec::with_capacity(req.entries.len());
    for entry in &req.entries {
        results.push(import_rules::apply_entry(
            ctx,
            req.cursor,
            entry,
            author_override,
            cfg,
        )?);
    }
    let head_seq = sync_log::head_seq(ctx.conn()?)?;
    Ok(ImportResponse { results, head_seq })
}
