//! `comemory install` — install the bundled, standalone comemory skills and
//! hooks in an agent host. The installer middle lives in `api::install`
//! (Binding Rule 1); this wrapper owns the argument shape and rendering.

use std::io::Write as _;
use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};

use crate::api;
use crate::config::Config;
use crate::output::json;
use crate::prelude::*;
use crate::utilities::context::Ctx;

const EXAMPLES: &str = "Examples:
  comemory install claude
  comemory install codex
  comemory install claude --dry-run --config-dir /tmp/claude-preview";

/// Supported native plugin managers. A CLI-only enum so `--help` and shell
/// completion can enumerate the hosts and accept the historical aliases;
/// `api::install::HOSTS` is the real source of truth.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Host {
    /// Claude Code skills and hooks.
    #[value(alias = "claude-hooks")]
    Claude,
    /// Codex skills and hooks.
    #[value(alias = "codex-hooks")]
    Codex,
}

impl Host {
    /// The canonical host name `api::install` validates against.
    fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }
}

/// Arguments to `comemory install` (independent of Git `install-hooks`).
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Agent host; installs both skills and lifecycle hooks.
    #[arg(value_enum)]
    pub host: Host,
    /// Preview the bundle destination without writing files.
    #[arg(long)]
    pub dry_run: bool,
    /// Override the host's user configuration directory.
    #[arg(long)]
    pub config_dir: Option<PathBuf>,
}

/// Install via `api::install::run` and render the report. `install` has no
/// `Paths`/db dependency, so `data_dir` resolves a throwaway `Ctx::lazy` that
/// is never opened.
pub fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = crate::config::Paths::new(crate::config::paths::resolve_data_dir(data_dir));
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let req = api::install::Request {
        host: a.host.name().to_string(),
        dry_run: a.dry_run,
        config_dir: a.config_dir,
    };
    let resp = api::install::run(&mut ctx, req)?;
    if json_flag {
        json::write(&resp)?;
    } else {
        emit(&resp)?;
    }
    Ok(())
}

/// Write the three-line human report: what happened, the plugin id, and the
/// toolu-migration note.
fn emit(resp: &api::install::Response) -> Result<()> {
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "{} comemory skills and hooks for {}: {}",
        if resp.dry_run {
            "Would install"
        } else {
            "Installed"
        },
        resp.host,
        resp.bundle.display()
    )?;
    writeln!(out, "Plugin: {}. Restart the host to load it.", resp.plugin)?;
    writeln!(
        out,
        "Migrating from toolu? Disable or uninstall comemory@toolu in {} to avoid duplicate hooks.",
        resp.host
    )?;
    Ok(())
}
