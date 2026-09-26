//! `comemory backup` — `create`, `restore` and `merge-erasures` (#256, B-4).
//! The cores live in `maintenance::backup` (Binding Rule 1); this file is
//! the clap surface, the restore's confirm gate and the rendering. CLI-only:
//! no `/api/v1` route replaces a server's own database on request.

use std::io::Write as _;
use std::path::PathBuf;

use clap::{Args as ClapArgs, Subcommand};

use crate::cli::load_config;
use crate::cli::output::json;
use crate::config::Config;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::domains::maintenance::backup::{self, RestoreRequest, Restored};
use crate::prelude::*;

const EXAMPLES: &str = "\
Examples:
  # Snapshot the database and memories/ into <data_dir>/backups/<timestamp>/
  comemory backup create

  # Restore it: a new stream epoch, every later erase merged back in
  comemory backup restore ~/.comemory/backups/20260925T101500.000Z --confirm

  # The erasure manifest was elsewhere: merge it and allow sync again
  comemory backup merge-erasures /mnt/safe/erasures.jsonl";

/// Arguments to `comemory backup` (nested subcommand required).
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// `create` / `restore` / `merge-erasures`.
    #[command(subcommand)]
    pub cmd: BackupCmd,
}

/// Nested `comemory backup <subcommand>`.
#[derive(Subcommand, Debug)]
pub enum BackupCmd {
    /// Snapshot the database and `memories/` (with `.trash/`) into a backup
    /// directory with a `backup.json` descriptor.
    Create(CreateArgs),
    /// Install a backup under a new stream epoch, merging every erase it
    /// predates; without an established erasure manifest it restores
    /// local-only and refuses sync until `merge-erasures`.
    Restore(RestoreArgs),
    /// Merge an established erasure manifest into the live store and allow
    /// sync again after a local-only restore.
    MergeErasures(MergeArgs),
}

/// Flags for `comemory backup create`.
#[derive(ClapArgs, Debug)]
pub struct CreateArgs {
    /// Write the backup here instead of `<data_dir>/backups/<UTC timestamp>/`.
    #[arg(long, value_name = "DIR")]
    pub out: Option<PathBuf>,
}

/// Flags for `comemory backup restore`.
#[derive(ClapArgs, Debug)]
pub struct RestoreArgs {
    /// The backup directory (`comemory.db` plus `memories/`).
    #[arg(value_name = "DIR")]
    pub dir: PathBuf,
    /// Confirm the restore: it replaces the live database and markdown
    /// (both are kept beside them as `*.pre-restore`).
    #[arg(long)]
    pub confirm: bool,
    /// Merge this erasure manifest instead of
    /// `<data_dir>/replica/erasures.jsonl`.
    #[arg(long, value_name = "FILE")]
    pub erasure_manifest: Option<PathBuf>,
}

/// Arguments to `comemory backup merge-erasures`.
#[derive(ClapArgs, Debug)]
pub struct MergeArgs {
    /// An erasure manifest (`erasures.jsonl`) holding at least the lines
    /// this engine's identity counts.
    #[arg(value_name = "FILE")]
    pub file: PathBuf,
}

/// Dispatch `comemory backup <subcommand>`.
///
/// # Errors
/// [`Error::ConfirmationRequired`] for a restore without `--confirm`;
/// everything the `maintenance::backup` cores return.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    let cfg = load_config(&paths)?;
    match a.cmd {
        BackupCmd::Create(c) => {
            let created = backup::create(&paths, &cfg, c.out)?;
            if json_flag {
                return json::write(&created);
            }
            writeln!(
                std::io::stdout().lock(),
                "backed up to {}: {} markdown files, epoch {}",
                created.dir,
                created.memory_files,
                created.descriptor.epoch
            )?;
            Ok(())
        }
        BackupCmd::Restore(r) => run_restore(&paths, &cfg, r, json_flag),
        BackupCmd::MergeErasures(m) => {
            let merged = backup::merge_erasures(&paths, &cfg, &m.file)?;
            if json_flag {
                return json::write(&merged);
            }
            writeln!(
                std::io::stdout().lock(),
                "merged {} erasure line(s), {} entities erased; manifest at {}{}",
                merged.lines,
                merged.entities_erased,
                merged.manifest,
                merged
                    .cleared
                    .map_or_else(String::new, |state| format!("; cleared `{state}`")),
            )?;
            Ok(())
        }
    }
}

/// `comemory backup restore`: the confirm gate, the restore, the report.
fn run_restore(paths: &Paths, cfg: &Config, r: RestoreArgs, json_flag: bool) -> Result<()> {
    if !r.confirm {
        return Err(Error::ConfirmationRequired(
            "restore replaces the live database and markdown; rerun with --confirm".to_string(),
        ));
    }
    let req = RestoreRequest {
        dir: r.dir,
        erasure_manifest: r.erasure_manifest,
    };
    let restored = backup::restore(paths, cfg, &req)?;
    if json_flag {
        return json::write(&restored);
    }
    write_restored(&mut std::io::stdout().lock(), &restored)
}

/// The text report of a restore.
fn write_restored(out: &mut impl std::io::Write, restored: &Restored) -> Result<()> {
    writeln!(
        out,
        "restored {}{}: epoch {}, device {}, {} erasure line(s) merged",
        restored.source,
        if restored.resumed {
            " (finished the pending swap)"
        } else {
            ""
        },
        restored.epoch,
        restored.device_id,
        restored.erasures_merged,
    )?;
    if let Some(state) = &restored.restore_state {
        writeln!(
            out,
            "restore state `{state}`: sync is refused until `comemory backup merge-erasures <manifest>`"
        )?;
    }
    for kept in &restored.pre_restore {
        writeln!(out, "previous state kept at {kept}")?;
    }
    Ok(())
}
