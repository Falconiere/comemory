//! `comemory erase` — permanently erase one memory or one document. The
//! erase itself lives in `maintenance::erase` (Binding Rule 1), shared with
//! `POST /api/v1/erase`; this file is the clap surface, the confirm gate and
//! the rendering.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::cli::load_config;
use crate::cli::output::json;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::domains::maintenance::erase;
use crate::prelude::*;
use crate::utilities::context::Ctx;

const EXAMPLES: &str = "\
Examples:
  # Permanently erase one memory, everywhere this engine holds its text
  comemory erase --memory a1b2c3d4 --confirm

  # Erase a document (local or pulled) by its shared id, as JSON
  comemory erase --document 0f3c9a1e5b7d4c2a8e6f1b3d5a7c9e0f --confirm --json";

/// Arguments to `comemory erase`: exactly one entity, and the confirmation.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
#[command(group = clap::ArgGroup::new("entity").required(true).args(["memory", "document"]))]
pub struct Args {
    /// Memory id to erase.
    #[arg(long, value_name = "ID")]
    pub memory: Option<String>,
    /// Shared id of a document to erase — one this engine shares, or a
    /// pulled copy.
    #[arg(long, value_name = "SHARED_ID")]
    pub document: Option<String>,
    /// Confirm the erase. It cannot be undone; without this flag nothing is
    /// erased.
    #[arg(long)]
    pub confirm: bool,
}

/// Erase the named entity, then report what went and which rollback
/// snapshots still hold its prior state.
///
/// # Errors
/// [`Error::ConfirmationRequired`] without `--confirm`; everything
/// `maintenance::erase::run` returns.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    if !a.confirm {
        return Err(Error::ConfirmationRequired(
            "erase is permanent; rerun with --confirm".to_string(),
        ));
    }
    let paths = Paths::new(resolve_data_dir(data_dir));
    let cfg = load_config(&paths)?;
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let report = erase::run(
        &mut ctx,
        erase::Request {
            memory: a.memory,
            document: a.document,
        },
    )?;
    if json_flag {
        return json::write(&report);
    }
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "erased {} {}: {} payloads erased, {} pending operations withdrawn, tombstone {}, \
         WAL {}",
        report.kind,
        report.key,
        report.payloads_erased,
        report.operations_withdrawn,
        if report.tombstoned {
            "journalled"
        } else {
            "not needed"
        },
        if report.wal_truncated {
            "truncated"
        } else {
            "still held by a reader"
        },
    )?;
    for snapshot in &report.snapshots_with_prior_state {
        writeln!(out, "still holds prior state: {snapshot}")?;
    }
    Ok(())
}
