//! `api::gc_policy` — `GET|PUT /api/v1/gc/policy` (console-api spec §9):
//! the two retention windows `comemory gc` reads (`prune.trash_retention_days`
//! and `prune.learning_retention_days`) plus the last recorded sweep.
//!
//! [`update`] is a `config.toml` writer, and follows the spec's "Config
//! patching" rule exactly: build the would-be [`Config`] in memory, run
//! `Config::validate` on it, and only then hand the supplied keys to
//! [`patch_config_file`]. An out-of-range window is therefore
//! `400 bad_request` with the file **byte-identical** — never a half-applied
//! policy that the next process start would refuse to load.
//!
//! Only the keys the caller actually supplied are written, so a `PUT` of
//! one window leaves the other's `config.toml` line (or its absence, and
//! hence its default) untouched.

use serde::{Deserialize, Serialize};

use crate::api::Ctx;
use crate::config::patch::{patch_config_file, section};
use crate::prelude::*;
use crate::store::gc_runs::{self, GcRunRow};

/// The `GET|PUT /api/v1/gc/policy` payload.
#[derive(Serialize, Debug)]
pub struct Policy {
    /// Days a soft-deleted memory stays in `memories/.trash/` before `gc`
    /// reaps it (`[prune] trash_retention_days`).
    pub trash_retention_days: u32,
    /// Days raw `retrieval_log` / `feedback_events` rows are kept
    /// (`[prune] learning_retention_days`). Named for the console, which
    /// calls this telemetry.
    pub telemetry_retention_days: u32,
    /// [`Policy::last_run`]'s timestamp, hoisted for a client that only
    /// wants "when did gc last run".
    pub last_run_at: Option<String>,
    /// The newest `gc_runs` row, or `None` when `gc` has never run (or the
    /// database does not exist yet).
    pub last_run: Option<GcRunRow>,
}

/// `PUT /api/v1/gc/policy` request. Both fields are optional: a `PUT`
/// carrying one window patches only that window.
#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct UpdateRequest {
    /// New `[prune] trash_retention_days`. Must be >= 1.
    #[serde(default)]
    pub trash_retention_days: Option<u32>,
    /// New `[prune] learning_retention_days`. Must be >= 1.
    #[serde(default)]
    pub telemetry_retention_days: Option<u32>,
}

/// `GET /api/v1/gc/policy` — the live windows plus the last sweep.
pub fn get(ctx: &mut Ctx<'_>) -> Result<Policy> {
    let trash = ctx.cfg.prune.trash_retention_days;
    let telemetry = ctx.cfg.prune.learning_retention_days;
    build(ctx, trash, telemetry)
}

/// `PUT /api/v1/gc/policy` — validate the patched windows in memory, write
/// only the supplied keys into `[prune]`, and return the new policy. See
/// the module doc for why validation strictly precedes the write.
pub fn update(ctx: &mut Ctx<'_>, req: UpdateRequest) -> Result<Policy> {
    let mut patched = ctx.cfg.clone();
    if let Some(days) = req.trash_retention_days {
        patched.prune.trash_retention_days = days;
    }
    if let Some(days) = req.telemetry_retention_days {
        patched.prune.learning_retention_days = days;
    }
    // `Config::validate` reports an out-of-range knob as `Error::Config`
    // (exit 78 on the CLI); over HTTP the caller sent it, so it is a
    // `400 bad_request`, not a server misconfiguration.
    let patched = patched
        .validate()
        .map_err(|e| Error::BadRequest(e.to_string()))?;
    patch_config_file(&ctx.paths.config_file(), |table| {
        let prune = section(table, "prune")?;
        if let Some(days) = req.trash_retention_days {
            prune.insert(
                "trash_retention_days".into(),
                toml::Value::Integer(i64::from(days)),
            );
        }
        if let Some(days) = req.telemetry_retention_days {
            prune.insert(
                "learning_retention_days".into(),
                toml::Value::Integer(i64::from(days)),
            );
        }
        Ok(())
    })?;
    build(
        ctx,
        patched.prune.trash_retention_days,
        patched.prune.learning_retention_days,
    )
}

/// Assemble a [`Policy`] from the two windows plus the newest `gc_runs`
/// row. The DB is read only when it already exists — a policy read must
/// never create (and migrate) `comemory.db` as a side effect, the same
/// invariant `api::gc::run` holds.
fn build(ctx: &mut Ctx<'_>, trash: u32, telemetry: u32) -> Result<Policy> {
    let last_run = if ctx.paths.db_path().exists() {
        gc_runs::newest(ctx.conn()?)?
    } else {
        None
    };
    Ok(Policy {
        trash_retention_days: trash,
        telemetry_retention_days: telemetry,
        last_run_at: last_run.as_ref().map(|row| row.at.clone()),
        last_run,
    })
}

#[cfg(test)]
#[path = "tests/gc_policy.rs"]
mod tests;
