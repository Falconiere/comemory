//! `qwick-memory conflicts` — list memory ids reachable from `<id>` via a single
//! `:ConflictsWith` edge. JSON output is a flat array; TTY output prints
//! one id per line. Returns an empty list when the memory has no recorded
//! conflicts (or does not exist in the graph at all).

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::graph::Graph;
use crate::output::json;
use crate::prelude::*;

/// Arguments to `qwick-memory conflicts`.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Memory id whose outgoing `:ConflictsWith` edges should be listed.
    pub id: String,
}

/// Query `:ConflictsWith` neighbours of `a.id` and render them.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let g = Graph::open(paths.graph_dir())?;
    let ids = g.conflicts_of(&a.id)?;

    if json_flag {
        json::write(&ids)?;
    } else {
        let mut out = std::io::stdout().lock();
        for id in &ids {
            writeln!(out, "{id}")?;
        }
    }
    Ok(())
}
