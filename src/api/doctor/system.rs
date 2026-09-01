//! `api::doctor::system::run` — `GET /api/v1/doctor/system` (console-api
//! spec §8): the **facts** half of doctor. Where `api::doctor::run` probes
//! (it writes a throwaway file, registers the tokenizer, and shells out to
//! `COMEMORY_EMBED_CMD`), this module only *reads*: versions, resolved
//! paths, on-disk file counts, and the two vector dims.
//!
//! Two invariants make it a facts read rather than a second doctor:
//!
//! 1. **It never runs the embed command** (AC-15). `embed_cmd` is reported
//!    verbatim as the configured string; nothing spawns it. A console
//!    polling this endpoint must not pay an embedder round-trip — and must
//!    not keep an embedder warm as a side effect of being open.
//! 2. **It never creates `comemory.db`.** Every DB-derived field is read
//!    only when the file already exists; on a fresh data dir
//!    `schema_version` is `None` and the vector dims fall back to the
//!    configured `[retrieval]` values rather than the (authoritative, but
//!    unreachable) `schema_meta` ones.

use serde::Serialize;

use crate::api::Ctx;
use crate::api::doctor::checks;
use crate::config::Paths;
use crate::config::env::env_parse;
use crate::prelude::*;
use crate::store::{migrate, vector};

/// The `GET /api/v1/doctor/system` payload: what this binary is, where its
/// data lives, and how much of it there is.
#[derive(Serialize, Debug)]
pub struct System {
    /// This binary's `CARGO_PKG_VERSION`.
    pub version: String,
    /// Applied `schema_meta.version`, or `None` when `comemory.db` does not
    /// exist yet (invariant 2 in the module doc).
    pub schema_version: Option<String>,
    /// The schema version this build expects (`migrate::CURRENT_VERSION`).
    pub current_schema_version: String,
    /// Resolved data directory.
    pub data_dir: String,
    /// Resolved `comemory.db` path (whether or not it exists).
    pub db_path: String,
    /// `comemory.db`'s size in bytes; `0` when it does not exist.
    pub db_bytes: u64,
    /// `*.md` files directly under `memories/`.
    pub markdown_files: u64,
    /// Files in `memories/.trash/` awaiting `gc`.
    pub trash_files: u64,
    /// Newest `comemory.db.pre-v{N}.bak` migration snapshot, if any.
    pub backup_path: Option<String>,
    /// That snapshot's size in bytes.
    pub backup_bytes: Option<u64>,
    /// The configured `COMEMORY_EMBED_CMD`, reported but NEVER run.
    pub embed_cmd: Option<String>,
    /// The operator's free-form embedder identifier (`embed_hint`).
    pub embed_hint: Option<String>,
    /// `memory_vec`'s dimension: `schema_meta` when the DB exists, else the
    /// configured `retrieval.memory_vector_dim`.
    pub memory_vec_dim: usize,
    /// `code_vec`'s dimension, resolved the same way.
    pub code_vec_dim: usize,
}

/// Build the system facts report. Opens `comemory.db` only when it already
/// exists (module doc, invariant 2) and never shells out to the embed
/// command (invariant 1).
pub fn run(ctx: &mut Ctx<'_>) -> Result<System> {
    let paths = ctx.paths;
    let (backup_path, backup_bytes) = newest_backup(paths)?;
    let mut system = System {
        version: env!("CARGO_PKG_VERSION").to_string(),
        schema_version: None,
        current_schema_version: migrate::CURRENT_VERSION.to_string(),
        data_dir: paths.data_dir().to_string_lossy().into_owned(),
        db_path: paths.db_path().to_string_lossy().into_owned(),
        db_bytes: checks::db_file_size(paths),
        markdown_files: count_files(&paths.memories_dir(), Some("md")),
        trash_files: count_files(&paths.trash_dir(), None),
        backup_path,
        backup_bytes,
        embed_cmd: env_parse::<String>("COMEMORY_EMBED_CMD")?,
        embed_hint: ctx.cfg.embed_hint.clone(),
        memory_vec_dim: ctx.cfg.retrieval.memory_vector_dim,
        code_vec_dim: ctx.cfg.retrieval.code_vector_dim,
    };
    if paths.db_path().exists() {
        let conn = ctx.conn()?;
        system.schema_version = Some(conn.query_row(
            "SELECT value FROM schema_meta WHERE key = 'version'",
            [],
            |r| r.get(0),
        )?);
        system.memory_vec_dim = vector::dim_memory(conn)?;
        system.code_vec_dim = vector::dim_code(conn)?;
    }
    Ok(system)
}

/// The newest `comemory.db.pre-v{N}.bak` snapshot beside the live DB, as
/// `(path, bytes)`. Shares `api::doctor::checks::newest_matching` with the
/// `doctor` report's "migration backup" check so the two can never name
/// different snapshots (Binding Rule 1).
fn newest_backup(paths: &Paths) -> Result<(Option<String>, Option<u64>)> {
    let db_path = paths.db_path();
    let Some(db_name) = db_path.file_name().and_then(|n| n.to_str()) else {
        return Ok((None, None));
    };
    let prefix = format!("{db_name}.pre-v");
    let newest = checks::newest_matching(paths.data_dir(), &prefix)?;
    Ok(match newest {
        Some((path, size)) => (Some(path.to_string_lossy().into_owned()), Some(size)),
        None => (None, None),
    })
}

/// Regular files directly under `dir`, optionally restricted to one
/// extension. A missing directory counts as zero rather than failing: a
/// facts read on a fresh data dir is a legitimate question, not an error.
fn count_files(dir: &std::path::Path, ext: Option<&str>) -> u64 {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    rd.flatten()
        .filter(|e| e.metadata().is_ok_and(|m| m.is_file()))
        .filter(|e| match ext {
            Some(want) => e.path().extension().is_some_and(|got| got == want),
            None => true,
        })
        .count() as u64
}

#[cfg(test)]
#[path = "../tests/doctor_system.rs"]
mod tests;
