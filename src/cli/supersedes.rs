//! `qwick-memory supersedes <new_id> <old_id>` — record that `new_id` supersedes
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
use crate::output::json;
use crate::prelude::*;

const EXAMPLES: &str = "\
Examples:
  qwick-memory supersedes e5f6a7b8 a1b2c3d4";

/// Arguments to `qwick-memory supersedes`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Memory id of the **new** decision (the one that supersedes).
    pub new_id: String,
    /// Memory id of the **old** decision (the one being superseded).
    pub old_id: String,
}

/// Record the `:Supersedes` edge and emit a success line (or a JSON envelope
/// with the new/old ids when `json` is set).
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let g = Graph::open(paths.graph_dir())?;
    g.add_supersedes(&a.new_id, &a.old_id)?;
    if json_flag {
        json::write(&serde_json::json!({
            "ok": true,
            "new": a.new_id,
            "old": a.old_id,
        }))?;
    } else {
        let mut out = std::io::stdout().lock();
        writeln!(out, "ok")?;
    }
    Ok(())
}
