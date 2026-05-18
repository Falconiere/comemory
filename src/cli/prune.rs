//! `qwick-memory prune` — detect (and optionally soft-delete) stale memories.
//!
//! Dry-run by default: the command reports candidate ids without touching
//! anything. With `--apply`, low-value candidates are soft-deleted via
//! [`MemoryStore::delete`] (which moves the file into `memories/.trash/`).
//! Orphan ids are reported either way; they live only in `.trash/` and are
//! reaped by `qwick-memory gc`.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;
use serde::Serialize;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::memory::MemoryStore;
use crate::output::json;
use crate::prelude::*;
use crate::prune::{low_value, orphans};

const EXAMPLES: &str = "\
Examples:
  qwick-memory prune --orphans
  qwick-memory prune --orphans --apply
  qwick-memory prune --low-value --below-quality 2 --unused-since 180 --apply";

/// Arguments to `qwick-memory prune`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Detect orphan entries in `memories/.trash/`.
    #[arg(long)]
    pub orphans: bool,
    /// Detect low-value memories (quality + unused + age gates).
    #[arg(long)]
    pub low_value: bool,
    /// Strict upper bound on quality for low-value matches.
    #[arg(long, default_value_t = 2)]
    pub below_quality: u8,
    /// Minimum age in days (since `created`) for low-value matches.
    #[arg(long, default_value_t = 180)]
    pub unused_since: u32,
    /// Perform soft-deletes instead of a dry-run.
    #[arg(long)]
    pub apply: bool,
}

/// Output schema for both JSON and TTY rendering. Lives at module scope so
/// downstream tooling can parse the JSON shape directly.
#[derive(Serialize, Debug)]
pub struct Report {
    pub orphans: Vec<String>,
    pub low_value: Vec<String>,
    pub applied: bool,
}

/// Detect stale memories per the requested gates and (optionally) soft-delete
/// low-value matches. Returns once the report is rendered.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;

    let orphan_ids = if a.orphans {
        orphans::detect(&paths)?
    } else {
        Vec::new()
    };
    let low_ids = if a.low_value {
        low_value::detect(&paths, a.below_quality, a.unused_since)?
    } else {
        Vec::new()
    };

    if a.apply {
        let store = MemoryStore::new(paths.clone());
        for id in &low_ids {
            // Best-effort: a missing id (e.g. concurrent delete) should not
            // abort the rest of the batch. Failures will resurface on the
            // next prune run because the file is still on disk.
            let _ = store.delete(id);
        }
    }

    let report = Report {
        orphans: orphan_ids,
        low_value: low_ids,
        applied: a.apply,
    };
    if json_flag {
        json::write(&report)?;
    } else {
        let mut out = std::io::stdout().lock();
        writeln!(out, "orphans:   {:?}", report.orphans)?;
        writeln!(out, "low_value: {:?}", report.low_value)?;
        writeln!(out, "applied:   {}", report.applied)?;
    }
    Ok(())
}
