//! `comemory rebuild` — atomically replace the SQLite mirror, preserving the
//! code index, by rebuilding from the on-disk markdown files. The rebuild
//! itself (tmp-DB build, `ATTACH` preservation copy, atomic rename) lives in
//! `api::rebuild` (Binding Rule 1); this file is the clap surface only.
//!
//! The command emits nothing on success — the `--json` flag has no output to
//! shape, exactly as before the extraction.

use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::api::{self, Ctx};
use crate::cli::resolve_data_dir;
use crate::config::Config;
use crate::config::paths::Paths;
use crate::prelude::*;

/// Arguments to `comemory rebuild`. Currently no flags — the command always
/// rebuilds the entire memory layer of the SQLite mirror from `memories/`
/// while preserving the code index. Wrapped in a struct so future opt-in
/// flags (e.g. `--keep-stats`, `--dry-run`) can land without breaking the
/// dispatcher signature.
#[derive(ClapArgs, Debug)]
pub struct Args;

/// Atomically rebuild the memory layer of `comemory.db` from markdown files
/// via `api::rebuild::run`, preserving any existing code index tables. On
/// any error the original DB is left untouched and the tmp file is removed.
///
/// `rebuild` never touches a caller-owned connection — it opens its own on
/// `comemory.db.rebuild.tmp` — so the `Ctx` here is `lazy` and never opened,
/// and its `Config` is the defaults (the command reads no ranking knobs).
pub async fn run(_args: Args, _json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    api::rebuild::run(&mut ctx, api::rebuild::Request {})
}
