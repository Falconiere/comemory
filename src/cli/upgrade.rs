//! `comemory upgrade` — move this binary to the newest release (or a pinned
//! one). Resolution, channel detection, and the swap live in
//! `crate::upgrade`; this wrapper parses flags and renders the report.
//! CLI-only by design: no `/api/v1` route (`serve::routes::meta::CLI_ONLY`).

use std::path::PathBuf;

use clap::Args as ClapArgs;
use owo_colors::OwoColorize;

use crate::output::json;
use crate::prelude::*;
use crate::upgrade::{self, Report, Request, Status};

const EXAMPLES: &str = "\
Examples:
  # Move to the newest release (a no-op when already there)
  comemory upgrade

  # Only report whether a newer release exists
  comemory upgrade --check

  # Pin a release; --force allows a reinstall or a downgrade
  comemory upgrade --version 0.19.0
  comemory upgrade --version 0.18.0 --force

  # Machine-readable
  comemory upgrade --check --json";

/// Arguments to `comemory upgrade`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Report the running and latest versions without installing anything.
    #[arg(long)]
    pub check: bool,
    /// Install this release instead of the latest (`0.19.0` or `v0.19.0`).
    #[arg(long, value_name = "VERSION")]
    pub version: Option<String>,
    /// Proceed even when the target is not newer than the running build
    /// (reinstall or downgrade).
    #[arg(long)]
    pub force: bool,
}

/// Run the upgrade and render its report. `data_dir` is unused: nothing
/// here touches the store. Under `--json` the installer runs quietly so
/// stdout carries only the report; on a TTY it paints its own progress.
pub async fn run(a: Args, json_flag: bool, _data_dir: Option<PathBuf>) -> Result<()> {
    let req = Request {
        check: a.check,
        version: a.version,
        force: a.force,
        quiet: json_flag,
    };
    let report = upgrade::run(&req)?;
    if json_flag {
        return json::write(&report);
    }
    render(&mut std::io::stdout().lock(), &report)
}

/// The two-line TTY report: outcome, then channel + path (dim), then any
/// hint.
fn render(out: &mut impl std::io::Write, r: &Report) -> Result<()> {
    let line = match r.status {
        Status::UpToDate => format!(
            "{} comemory {} is up to date",
            "\u{2713}".green(),
            r.current.bold()
        ),
        Status::Available => format!(
            "{} update available: {} \u{2192} {}",
            "\u{2191}".cyan(),
            r.current,
            r.target.bold().green()
        ),
        Status::Upgraded => format!(
            "{} upgraded comemory {} \u{2192} {}",
            "\u{2713}".green(),
            r.current,
            r.target.bold()
        ),
        Status::Installed => format!(
            "{} installed comemory {} (was {})",
            "\u{2713}".green(),
            r.target.bold(),
            r.current
        ),
    };
    writeln!(out, "{line}")?;
    let detail = format!("  {} \u{b7} {}", r.channel.label(), r.exe.display());
    writeln!(out, "{}", detail.dimmed())?;
    if let Some(hint) = &r.hint {
        writeln!(out, "  {hint}")?;
    }
    Ok(())
}
