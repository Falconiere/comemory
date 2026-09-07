//! `POST /sync/import` — apply a batch of wire entries (rules 1–10).

use crate::api::Ctx;
use crate::api::sync::import_rules;
use crate::api::sync::{ImportRequest, ImportResponse};
use crate::prelude::*;
use crate::store::sync_log;

const MAX_ENTRIES: usize = 500;

/// Apply `req.entries` in order, one transaction per entry.
pub fn run(
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
