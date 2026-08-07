//! `comemory ingest-code` — read pre-embedded code symbol rows from stdin
//! (one JSON object per line) and mirror them into `code_symbols`,
//! `code_fts`, and `code_vec` via the shared [`api::ingest_code::run`]
//! middle (Binding Rule 1).
//!
//! Pairs with `comemory index-code --extract`, which emits the same JSONL
//! shape minus the `embedding` field.

use std::io::Read as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::api::{self, Ctx};
use crate::cli::load_config;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::prelude::*;
use crate::store::connection;

const EXAMPLES: &str = "\
Examples:
  # Pipe pre-embedded JSONL from your embedder into the SQLite store
  comemory index-code --repo myrepo --path . --extract \\
    | embed-snippets \\
    | comemory ingest-code";

/// Arguments to `comemory ingest-code`. Currently no flags — input is the
/// stdin stream of JSONL rows. Wrapped in a struct so future flags can land
/// without breaking the dispatcher signature.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args;

/// Read all of stdin into a buffer and hand it to [`api::ingest_code::run`].
pub async fn run(_args: Args, _json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let mut body = String::new();
    std::io::stdin()
        .lock()
        .read_to_string(&mut body)
        .map_err(Error::Io)?;
    let cfg = load_config(&paths)?;
    let mut conn = connection::open(paths.db_path())?;
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    api::ingest_code::run(&mut ctx, &body)?;
    Ok(())
}
