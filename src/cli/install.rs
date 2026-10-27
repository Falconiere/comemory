//! `comemory install` — install the bundled, standalone comemory skills and
//! hooks in an agent host. The installer middle lives in
//! `domains::integrations::install`
//! (Binding Rule 1); this wrapper owns the argument shape and rendering.

use std::io::Write as _;
use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};

use crate::cli::output::json;
use crate::config::Config;
use crate::domains::integrations;
use crate::prelude::*;
use crate::utilities::context::Ctx;

const EXAMPLES: &str = "Examples:
  comemory install claude
  comemory install muse
  comemory install cursor-agent --dry-run --config-dir /tmp/cursor-preview";

/// Supported agent hosts. A CLI-only enum so `--help` and shell completion
/// can enumerate the hosts and accept the historical aliases;
/// `domains::integrations::install::host::TABLE` is the real source of truth.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Host {
    /// Claude Code skills, hooks and MCP server.
    #[value(alias = "claude-hooks")]
    Claude,
    /// Codex skills, hooks and MCP server.
    #[value(alias = "codex-hooks")]
    Codex,
    /// Muse skills, hooks and MCP server (approve hooks with `muse plugins approve comemory`).
    #[value(alias = "muse-hooks")]
    Muse,
    /// Cursor skills and MCP config (hooks are a follow-up).
    #[value(alias = "cursor")]
    #[value(alias = "cursor-hooks")]
    CursorAgent,
    /// Hermes skills (no hooks or MCP story on this host).
    #[value(alias = "hermes-hooks")]
    Hermes,
}

impl Host {
    /// The canonical host name `domains::integrations::install` validates against.
    fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Muse => "muse",
            Self::CursorAgent => "cursor-agent",
            Self::Hermes => "hermes",
        }
    }
}

/// Arguments to `comemory install` (independent of Git `install-hooks`).
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Agent host; installs skills plus whatever hooks and MCP wiring the host supports.
    #[arg(value_enum)]
    pub host: Host,
    /// Preview the bundle destination without writing files.
    #[arg(long)]
    pub dry_run: bool,
    /// Override the host's user configuration directory.
    #[arg(long)]
    pub config_dir: Option<PathBuf>,
}

/// Install via `domains::integrations::install::run` and render the report.
/// `install` has no
/// `Paths`/db dependency, so `data_dir` resolves a throwaway `Ctx::lazy` that
/// is never opened.
pub fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = crate::config::Paths::new(crate::config::paths::resolve_data_dir(data_dir));
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);
    let req = integrations::install::Request {
        host: a.host.name().to_string(),
        dry_run: a.dry_run,
        config_dir: a.config_dir,
    };
    let resp = integrations::install::run(&mut ctx, req)?;
    if json_flag {
        json::write(&resp)?;
    } else {
        emit(&resp)?;
    }
    Ok(())
}

/// Write the human report: what was installed where, the plugin id for a
/// plugin-manager host, then the host's own next steps.
fn emit(resp: &integrations::install::Response) -> Result<()> {
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "{} comemory {} for {}: {}",
        if resp.dry_run {
            "Would install"
        } else {
            "Installed"
        },
        resp.delivers,
        resp.host,
        resp.bundle.display()
    )?;
    if let Some(plugin) = resp.plugin {
        writeln!(out, "Plugin: {plugin}.")?;
    }
    for step in &resp.next_steps {
        writeln!(out, "{step}")?;
    }
    Ok(())
}
