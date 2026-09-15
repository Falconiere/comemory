//! Install the bundled, standalone comemory skills and hooks in an agent host.
use std::io::Write as _;
use std::path::PathBuf;
use std::process::Command;

use clap::{Args as ClapArgs, ValueEnum};
use serde_json::json;

use crate::prelude::*;

mod bundle;

const EXAMPLES: &str = "Examples:
  comemory install claude
  comemory install codex
  comemory install claude --dry-run --config-dir /tmp/claude-preview";

/// Supported native plugin managers.
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
    fn config_dir(self, explicit: Option<PathBuf>) -> Result<PathBuf> {
        let (key, directory) = match self {
            Self::Claude => ("CLAUDE_CONFIG_DIR", ".claude"),
            Self::Codex => ("CODEX_HOME", ".codex"),
        };
        let configured = explicit.or(crate::config::env::env_parse::<PathBuf>(key)?);
        let root = if let Some(root) = configured {
            root
        } else {
            crate::config::env::env_parse::<PathBuf>("HOME")?
                .ok_or_else(|| Error::Usage("HOME is unset; pass --config-dir".into()))?
                .join(directory)
        };
        Ok(std::path::absolute(root)?)
    }

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

/// Extract the embedded integration and register it with the native host CLI.
pub fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let root = crate::config::paths::resolve_data_dir(data_dir)
        .join("integrations")
        .join(env!("CARGO_PKG_VERSION"));
    let root = std::path::absolute(root)?;
    let config_dir = a.host.config_dir(a.config_dir)?;
    let name = a.host.name();
    if !a.dry_run {
        install(a.host, &config_dir, &root)?;
    }
    let report = json!({
        "marketplace": root.parent(),
        "host": name, "plugin": "comemory@comemory", "bundle": root,
        "config_dir": config_dir, "dry_run": a.dry_run,
        "installed": !a.dry_run,
    });
    if json_flag {
        crate::output::json::write(&report)?;
    } else {
        let mut out = std::io::stdout().lock();
        writeln!(
            out,
            "{} comemory skills and hooks for {name}: {}",
            if a.dry_run {
                "Would install"
            } else {
                "Installed"
            },
            root.display()
        )?;
        writeln!(
            out,
            "Plugin: comemory@comemory. Restart the host to load it."
        )?;
        writeln!(
            out,
            "Migrating from toolu? Disable or uninstall comemory@toolu in {name} to avoid duplicate hooks."
        )?;
    }
    Ok(())
}

fn install(host: Host, config_dir: &std::path::Path, root: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(config_dir)?;
    run_host(host, Some(config_dir), &["--version"])?;
    for dependency in ["bash", "git", "jq", "python3"] {
        let status = Command::new(dependency).arg("--version").output()?;
        if !status.status.success() {
            return Err(Error::Unavailable(format!(
                "{dependency} is required for agent hooks"
            )));
        }
    }
    bundle::extract(root)?;
    let marketplace = root
        .parent()
        .ok_or_else(|| Error::Usage("bundle needs a parent".into()))?;
    let path = marketplace
        .to_str()
        .ok_or_else(|| Error::Usage("integration path must be UTF-8".into()))?;
    run_host(
        host,
        Some(config_dir),
        &["plugin", "marketplace", "add", path],
    )?;
    let action = match host {
        Host::Claude => "install",
        Host::Codex => "add",
    };
    run_host(
        host,
        Some(config_dir),
        &["plugin", action, "comemory@comemory"],
    )?;
    if matches!(host, Host::Claude) {
        run_host(
            host,
            Some(config_dir),
            &["plugin", "update", "comemory@comemory"],
        )?;
    }
    Ok(())
}

fn run_host(host: Host, config_dir: Option<&std::path::Path>, args: &[&str]) -> Result<()> {
    let mut command = Command::new(host.name());
    command.args(args);
    if let Some(dir) = config_dir {
        command.env(
            match host {
                Host::Claude => "CLAUDE_CONFIG_DIR",
                Host::Codex => "CODEX_HOME",
            },
            dir,
        );
    }
    let output = command.output().map_err(|error| {
        Error::Unavailable(format!(
            "{} {}: {error}. Install the host CLI before retrying.",
            host.name(),
            args.join(" ")
        ))
    })?;
    if !output.status.success() {
        return Err(Error::Unavailable(format!(
            "{} {} failed ({}): {} {}",
            host.name(),
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        )));
    }
    Ok(())
}
