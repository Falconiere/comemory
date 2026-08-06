//! `api::doctor::{Request, run}` — the shared middle of `comemory doctor` /
//! `GET /api/v1/doctor`: probe data-dir + DB writability without ever
//! creating the DB as an unwanted side effect. Moved out of
//! `cli::doctor::run` (Binding Rule 1).
//!
//! [`run`] checks writability by hand **before** ever calling [`Ctx::conn`],
//! so an unwritable data dir never reaches `store::connection::open`'s own
//! `CREATE`+migrate path; once writable, `Ctx::conn` opens (and may create)
//! the DB exactly as the original CLI's direct `connection::open` call did.

use serde::{Deserialize, Serialize};

use crate::api::Ctx;
use crate::config::Paths;
use crate::prelude::*;
use crate::store::migrate;

/// `comemory doctor` / `GET /api/v1/doctor` request. No fields — `doctor`
/// takes no arguments; the empty struct still derives `Deserialize` +
/// `deny_unknown_fields` so it fits the uniform `api::<cmd>::Request` shape.
#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct Request {}

/// JSON shape emitted under `--json` and the `/api/v1/doctor` `data` field.
#[derive(Serialize, Debug)]
pub struct Report {
    /// Resolved data directory (after `--data-dir` / `COMEMORY_DATA_DIR`
    /// fallback).
    pub data_dir: String,
    /// `true` when `comemory.db` exists and is writable.
    pub db_writable: bool,
    /// Applied schema version from `schema_meta.version`.
    pub schema_version: String,
    /// `true` when `vec_version()` returns a string, i.e. the
    /// sqlite-vec extension was loaded into this connection.
    pub sqlite_vec_loaded: bool,
    /// Free-form identifier of the embedder the operator configured
    /// (e.g. `ollama:nomic-embed-text`). `None` when `COMEMORY_EMBED_HINT`
    /// is not set.
    pub embed_hint: Option<String>,
}

/// Build the doctor report. See the module doc for the writability-probe
/// ordering that keeps this from ever creating a DB in an unwritable
/// location.
pub fn run(ctx: &mut Ctx<'_>, _req: Request) -> Result<Report> {
    let paths = ctx.paths;
    let embed_hint = ctx.cfg.embed_hint.clone();
    if !probe_writable(paths) {
        return Ok(unwritable_report(paths, embed_hint));
    }
    let report = writable_report(ctx, embed_hint)?;
    if report.schema_version != migrate::CURRENT_VERSION {
        return Err(Error::Migration(format!(
            "schema version {} != expected {}",
            report.schema_version,
            migrate::CURRENT_VERSION
        )));
    }
    Ok(report)
}

/// `true` when `comemory.db` is writable. An existing DB is probed with a
/// plain read+write open (no `create`); a missing one is probed via a
/// throwaway file in the data dir (the parent was already ensured by the
/// caller's `paths.ensure_dirs()`).
fn probe_writable(paths: &Paths) -> bool {
    let db_path = paths.db_path();
    if db_path.exists() {
        return std::fs::OpenOptions::new()
            .write(true)
            .open(&db_path)
            .is_ok();
    }
    let probe = paths.data_dir().join(".comemory.doctor.probe");
    let ok = std::fs::write(&probe, b"").is_ok();
    if ok {
        let _ = std::fs::remove_file(&probe);
    }
    ok
}

/// The partial report emitted when `comemory.db` is not writable — no
/// connection was opened, so schema/vec fields report their "unknown"
/// defaults rather than lying about a DB that was never probed.
fn unwritable_report(paths: &Paths, embed_hint: Option<String>) -> Report {
    Report {
        data_dir: paths.data_dir().to_string_lossy().into_owned(),
        db_writable: false,
        schema_version: "unknown".into(),
        sqlite_vec_loaded: false,
        embed_hint,
    }
}

/// The full report, opening the connection (and, on a brand-new writable
/// data dir, creating + migrating the DB) via [`Ctx::conn`].
fn writable_report(ctx: &mut Ctx<'_>, embed_hint: Option<String>) -> Result<Report> {
    let data_dir = ctx.paths.data_dir().to_string_lossy().into_owned();
    let conn = ctx.conn()?;
    let schema_version: String = conn.query_row(
        "SELECT value FROM schema_meta WHERE key = 'version'",
        [],
        |r| r.get(0),
    )?;
    let sqlite_vec_loaded = conn
        .query_row("SELECT vec_version()", [], |r| r.get::<_, String>(0))
        .is_ok();
    Ok(Report {
        data_dir,
        db_writable: true,
        schema_version,
        sqlite_vec_loaded,
        embed_hint,
    })
}
