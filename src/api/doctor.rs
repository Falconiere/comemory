//! `api::doctor::{Request, run}` — the shared middle of `comemory doctor` /
//! `GET /api/v1/doctor`: probe data-dir + DB writability without ever
//! creating the DB as an unwanted side effect. Moved out of
//! `cli::doctor::run` (Binding Rule 1).
//!
//! [`run`] checks writability by hand **before** ever calling [`Ctx::conn`],
//! so an unwritable data dir never reaches `store::connection::open`'s own
//! `CREATE`+migrate path; once writable, `Ctx::conn` opens (and may create)
//! the DB exactly as the original CLI's direct `connection::open` call did.
//!
//! ## Forward-compat fallback
//!
//! `doctor` is the one command whose job is to explain a broken state, so
//! unlike every other command it does not let a database written by a
//! *newer* comemory (refused by `store::migrate::preflight` with
//! `Error::SchemaTooNew`) bubble up as a bare exit-70 failure. [`run`] tries
//! [`Ctx::conn`] **first** — that ordering is load-bearing, not incidental:
//! a read-only open can never `CREATE` a database, so making it the primary
//! path would break every one of this module's own fresh-dir tests (see
//! `tests/doctor.rs` and the crate-root `cli__doctor` suite). Only on
//! `Err(Error::SchemaTooNew(_))` does it open a **second**, read-only
//! connection and report the unknown `schema_meta` marker keys as a field
//! instead of propagating the error. That second connection never runs
//! `preflight` or `migrate::run` — it is a plain read-only `rusqlite::Connection`.
//!
//! This fallback is deliberately narrow: a *genuinely* broken migration
//! (its SQL fails to re-apply, or a mandatory pre-upgrade snapshot fails
//! ahead of a `Destructive` migration) surfaces as `Error::Migration`, a
//! distinct variant `run` does **not** catch here — it propagates through
//! the final `Err(e) => Err(e)` arm like it does for every other command.
//! Swallowing `Error::Migration` here too once reported a clean bill of
//! health on a database `comemory list` itself refused to open.

use serde::{Deserialize, Serialize};

use crate::api::Ctx;
use crate::config::Paths;
use crate::prelude::*;
use crate::store::migrate;
use crate::store::migrate::preflight;

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
    /// Applied schema version from `schema_meta.version` — but NOT always:
    /// this is the literal string `"unknown"` (not a real version) both
    /// when `comemory.db` is not writable (no connection was ever opened
    /// to read it, see [`unwritable_report`]) and, on the forward-compat
    /// fallback path (see the module doc), when the stored value cannot be
    /// read at all or reads back as this build's own [`migrate::CURRENT_VERSION`]
    /// despite an unknown marker being present — a value that would
    /// otherwise misleadingly claim the database matches this build's
    /// schema (see [`stored_schema_version`]).
    pub schema_version: String,
    /// `true` when `vec_version()` returns a string, i.e. the
    /// sqlite-vec extension was loaded into this connection.
    pub sqlite_vec_loaded: bool,
    /// Free-form identifier of the embedder the operator configured
    /// (e.g. `ollama:nomic-embed-text`). `None` when `COMEMORY_EMBED_HINT`
    /// is not set.
    pub embed_hint: Option<String>,
    /// `schema_meta` marker keys found in the database that this build does
    /// not recognize. Empty in the normal case; non-empty only when
    /// `Ctx::conn` was refused with `Error::SchemaTooNew` (the database was
    /// written by a newer comemory) and `run` fell back to a read-only
    /// probe — see the module doc's "Forward-compat fallback".
    pub unknown_migration_keys: Vec<String>,
}

/// Build the doctor report. See the module doc for the writability-probe
/// ordering that keeps this from ever creating a DB in an unwritable
/// location, and for the read-only forward-compat fallback below.
pub fn run(ctx: &mut Ctx<'_>, _req: Request) -> Result<Report> {
    let paths = ctx.paths;
    let embed_hint = ctx.cfg.embed_hint.clone();
    if !probe_writable(paths) {
        return Ok(unwritable_report(paths, embed_hint));
    }
    match writable_report(ctx, embed_hint.clone()) {
        Ok(report) if report.schema_version != migrate::CURRENT_VERSION => {
            Err(Error::Migration(format!(
                "schema version {} != expected {}",
                report.schema_version,
                migrate::CURRENT_VERSION
            )))
        }
        Ok(report) => Ok(report),
        // The primary path already ran (see above) and was refused for the
        // forward-compat reason specifically. Fall back to a read-only
        // probe rather than propagating the failure — see the module doc's
        // "Forward-compat fallback". Every other error (including a
        // genuinely broken `Error::Migration`) falls through to the arm
        // below and propagates.
        Err(Error::SchemaTooNew(_)) => forward_compat_report(paths, embed_hint),
        Err(e) => Err(e),
    }
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
        unknown_migration_keys: Vec::new(),
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
        unknown_migration_keys: Vec::new(),
    })
}

/// The fallback report built when [`Ctx::conn`] was refused with
/// `Error::SchemaTooNew` — see the module doc's "Forward-compat fallback".
/// Opens a **second**, plain `rusqlite::Connection` in
/// [`rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY`] mode: never `preflight`,
/// never `migrate::run`, and (being read-only) incapable of creating a
/// database even by accident.
///
/// `sqlite-vec`'s `vec_version()` is still reachable here even though this
/// connection never goes through `store::connection::open`: it is
/// registered once per process as a SQLite auto-extension by the primary
/// `Ctx::conn` attempt that already ran (and failed only once past that
/// registration, inside `preflight`), so every subsequent connection in
/// this process — including this one — inherits it.
fn forward_compat_report(paths: &Paths, embed_hint: Option<String>) -> Result<Report> {
    let data_dir = paths.data_dir().to_string_lossy().into_owned();
    let conn = rusqlite::Connection::open_with_flags(
        paths.db_path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let sqlite_vec_loaded = conn
        .query_row("SELECT vec_version()", [], |r| r.get::<_, String>(0))
        .is_ok();
    let unknown_migration_keys = unknown_keys(&conn)?;
    let schema_version = stored_schema_version(&conn, migrate::CURRENT_VERSION);
    Ok(Report {
        data_dir,
        db_writable: true,
        schema_version,
        sqlite_vec_loaded,
        embed_hint,
        unknown_migration_keys,
    })
}

/// The raw `schema_meta.version` string, unless it equals `current` — this
/// function is only ever called from the forward-compat path (see
/// [`forward_compat_report`]), where an unknown marker is already known to
/// exist, so a stored version that still reads `current` cannot actually
/// describe a database at *this* build's current schema. Applies the same
/// "the reported version must reflect what was really applied" invariant
/// [`run`]'s primary path enforces (its own `schema_version !=
/// migrate::CURRENT_VERSION` guard), rather than trusting a value that
/// would otherwise print as deceptively up to date. Falls back to the same
/// `"unknown"` sentinel [`unwritable_report`] uses when the version cannot
/// be read at all, or cannot be trusted here.
fn stored_schema_version(conn: &rusqlite::Connection, current: &str) -> String {
    let stored = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'version'",
            [],
            |r| r.get::<_, String>(0),
        )
        .unwrap_or_else(|_| "unknown".to_string());
    if stored == current {
        "unknown".to_string()
    } else {
        stored
    }
}

/// `applied_keys(conn) \ expected_markers()` — the `schema_meta` marker
/// keys this build does not recognize. Reuses
/// `store::migrate::preflight`'s own set-derivation rather than
/// re-deriving it, so the two can never disagree about what "unknown"
/// means.
fn unknown_keys(conn: &rusqlite::Connection) -> Result<Vec<String>> {
    let applied = preflight::applied_keys(conn)?;
    let expected = preflight::expected_markers();
    Ok(applied
        .into_iter()
        .filter(|k| !expected.contains(k.as_str()))
        .collect())
}

#[cfg(test)]
#[path = "tests/doctor.rs"]
mod tests;
