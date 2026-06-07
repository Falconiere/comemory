//! `comemory delete` — soft-delete a memory by id (moves the file into
//! `memories/.trash/`).

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;
use serde::Serialize;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::memory::MemoryStore;
use crate::prelude::*;

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
    /// 12-hex memory id to delete.
    pub id: String,
}

/// JSON shape emitted under `--json`.
#[derive(Serialize)]
struct Output {
    deleted: String,
}

/// Soft-delete the memory and report the affected id.
pub async fn run(a: Args, json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    // `ensure_dirs` guarantees `memories/` exists before the store enumerates
    // it — without this a fresh data dir surfaces ENOENT instead of the
    // intended "memory not found" message from `MemoryStore::delete`.
    paths.ensure_dirs()?;
    let removed = MemoryStore::new(paths).delete(&a.id)?;
    let mut out = std::io::stdout().lock();
    if json {
        let output = Output {
            deleted: removed.frontmatter.id.clone(),
        };
        writeln!(out, "{}", serde_json::to_string(&output)?)?;
    } else {
        writeln!(out, "deleted {}", removed.frontmatter.id)?;
    }
    Ok(())
}
