//! `comemory delete` — soft-delete a memory by id (moves the file into
//! `memories/.trash/`, stamps `deleted_at` in `comemory.db`, and removes
//! all touching graph edges + the FTS5 row). The deletion itself lives in
//! `domains::memories::delete`; this module owns the clap flags, the inline
//! cloud push and the output rendering.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::cli::load_config;
use crate::cli::off_runtime::off_runtime;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::prelude::*;
use crate::store::connection;
use crate::utilities::context::Ctx;

const EXAMPLES: &str = "\
Examples:
  # Soft-delete by id (moves to memories/.trash/)
  comemory delete a1b2c3d4

  # JSON output for scripting
  comemory delete a1b2c3d4 --json";

/// Arguments to `comemory delete`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// 8-hex memory id to delete.
    pub id: String,
}

/// Soft-delete the memory and report the affected id. Parses `Args`, opens
/// the store, and delegates to [`crate::domains::memories::delete::run`]
/// (Binding Rule 1) —
/// shared with `DELETE /api/v1/memories/{id}`.
pub async fn run(a: Args, json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    // `ensure_dirs` guarantees `memories/` exists before the store enumerates
    // it — without this a fresh data dir surfaces ENOENT instead of the
    // intended "memory not found" message from `MemoryStore::delete`.
    paths.ensure_dirs()?;

    let cfg = load_config(&paths)?;
    let mut conn = connection::open(paths.db_path())?;
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let output = crate::domains::memories::delete::run(&mut ctx, &a.id)?;
    // A tombstone is a change like any other: push it inline so the console
    // and every other device see the delete without waiting for a sync.
    //
    // Both drops are load-bearing and ordered: `ctx` borrows `conn` mutably,
    // and `push_on_save` opens its own connection — so the borrow has to end
    // before the connection does, and the connection before the push. The
    // compiler enforces the first; the second is why `conn` is dropped early
    // rather than at end of scope.
    drop(ctx);
    drop(conn);
    off_runtime(|| {
        crate::domains::sync::push_on_save::after_write_best_effort(&paths, &cfg);
        Ok(())
    })?;

    let mut out = std::io::stdout().lock();
    if json {
        writeln!(out, "{}", serde_json::to_string(&output)?)?;
    } else {
        writeln!(out, "deleted {}", output.deleted)?;
    }
    Ok(())
}
