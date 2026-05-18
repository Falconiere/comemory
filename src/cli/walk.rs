//! `qwick-memory walk` — graph walk over the memory layer. Today only the
//! `--edge supersedes` form is wired; it traverses the `:Supersedes` chain
//! out to `--depth` hops and emits the reachable memory ids. JSON output is
//! a flat array of ids; TTY output prints one id per line.
//!
//! Future edges (`conflicts`, `relates`, `references-symbol`, …) will be
//! added behind the same dispatch.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::graph::Graph;
use crate::output::json;
use crate::prelude::*;

/// Arguments to `qwick-memory walk`.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Memory id to start walking from.
    #[arg(long)]
    pub from: String,
    /// Edge kind to traverse. Currently only `supersedes` is supported.
    #[arg(long, default_value = "supersedes")]
    pub edge: String,
    /// Maximum hop depth. Clamped to at least 1 by the underlying query.
    #[arg(long, default_value_t = 5)]
    pub depth: u32,
}

/// Walk the requested edge and render the reachable ids.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let g = Graph::open(paths.graph_dir())?;
    let ids = match a.edge.as_str() {
        "supersedes" => g.supersedes_chain(&a.from, a.depth)?,
        other => return Err(Error::Other(format!("unsupported edge: {other}"))),
    };

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
