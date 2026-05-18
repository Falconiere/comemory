//! `qwick supersedes <new_id> <old_id>` — record that `new_id` supersedes
//! `old_id` in the kuzu memory graph. Idempotent: calling repeatedly with the
//! same pair re-uses the existing edge (the underlying `MERGE`).
//!
//! Note: the current implementation does **not** verify that both ids refer
//! to existing `Memory` nodes. If either id is missing the `MATCH` simply
//! matches zero rows and the `MERGE` is a no-op — the command still prints
//! `ok`. This matches the v1 semantics of [`Graph::add_supersedes`]; tighter
//! validation is deferred.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::graph::Graph;
use crate::prelude::*;

/// Arguments to `qwick supersedes`.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Memory id of the **new** decision (the one that supersedes).
    pub new_id: String,
    /// Memory id of the **old** decision (the one being superseded).
    pub old_id: String,
}

/// Record the `:Supersedes` edge and emit a success line.
pub async fn run(a: Args, _json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let g = Graph::open(paths.graph_dir())?;
    g.add_supersedes(&a.new_id, &a.old_id)?;
    let mut out = std::io::stdout().lock();
    writeln!(out, "ok")?;
    Ok(())
}
