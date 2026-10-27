//! `domains::integrations::install::{Request, Response, run}` — the shared middle of
//! `comemory install <host>`: extract the embedded agent integration, write
//! `<bundle>/plugins/comemory/.mcp.json` with this binary's absolute path,
//! and register the plugin with the host's native plugin manager. Moved out
//! of `cli::install::run` (Binding Rule 1) so `setup` drives the same
//! installation instead of duplicating the host probe.
//!
//! The host is a validated string, not a clap enum, for the same reason
//! `crate::domains::code::hooks::Request::enable` is. Conn-free like
//! `domains::code::install_hooks` — [`run`] never calls `Ctx::conn`.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::prelude::*;
use crate::utilities::context::Ctx;

mod bundle;
mod cursor;
mod dirs;
mod hermes;

/// The agent hosts `comemory install` knows how to register a plugin with.
/// Each name is the invocable host binary where one exists: `run_host`
/// executes `Command::new(host)`, so registration and the setup presence
/// probe both depend on it.
pub const HOSTS: &[&str] = &["claude", "codex", "muse", "cursor-agent", "hermes"];

/// `comemory install` request. `host` is one of [`HOSTS`].
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Agent host name; installs both skills and lifecycle hooks.
    pub host: String,
    /// Preview the bundle destination without writing files.
    #[serde(default)]
    pub dry_run: bool,
    /// Override the host's user configuration directory.
    #[serde(default)]
    pub config_dir: Option<PathBuf>,
}

/// Report of one `comemory install` run.
#[derive(Serialize, Debug)]
pub struct Response {
    /// The local marketplace directory the bundle's parent forms.
    pub marketplace: Option<PathBuf>,
    /// The validated host name.
    pub host: String,
    /// The plugin id registered with the host (`comemory@comemory`).
    pub plugin: &'static str,
    /// Where the integration bundle was extracted.
    pub bundle: PathBuf,
    /// Where `.mcp.json` was written (or would be written under `--dry-run`).
    pub mcp_manifest: PathBuf,
    /// The host configuration directory used.
    pub config_dir: PathBuf,
    /// Whether this was a preview.
    pub dry_run: bool,
    /// Whether files were actually written.
    pub installed: bool,
}

/// The plugin id every host registers the bundle under.
const PLUGIN: &str = "comemory@comemory";

/// Where a successful install records that `host` was registered at this
/// comemory version.
///
/// The extracted bundle itself is **shared** — every host gets the same
/// `<data_dir>/integrations/<version>` tree — so its existence says nothing
/// about which hosts had the plugin registered with their own CLI. This
/// per-host marker is that missing fact, and it is version-scoped so an
/// upgrade correctly reports the host as needing a reinstall.
pub fn marker_path(data_dir: &Path, host: &str) -> PathBuf {
    data_dir
        .join("integrations")
        .join(env!("CARGO_PKG_VERSION"))
        .join(format!(".installed-{host}"))
}

/// Whether `host` has this comemory version's plugin registered.
pub fn is_installed(data_dir: &Path, host: &str) -> bool {
    marker_path(data_dir, host).exists()
}

/// External programs the generated agent hooks shell out to. Probed before
/// anything is written so a missing one fails the install rather than
/// surfacing later as a silently broken hook.
const HOOK_DEPENDENCIES: &[&str] = &["bash", "git", "jq", "python3"];

/// Validate `name` against [`HOSTS`], returning the canonical `&'static str`
/// so downstream code never carries an unvalidated host around.
///
/// # Errors
/// [`Error::Usage`] naming the offending value and the known hosts.
pub fn validate_host(name: &str) -> Result<&'static str> {
    HOSTS
        .iter()
        .copied()
        .find(|known| *known == name)
        .ok_or_else(|| {
            Error::Usage(format!(
                "unknown agent host `{name}` (known: {})",
                HOSTS.join(", ")
            ))
        })
}

/// The environment variable a host reads its configuration directory from.
/// `None` for hosts without one: their config dir derives from `HOME`
/// unless explicitly overridden.
fn config_env_key(host: &str) -> Option<&'static str> {
    match host {
        "claude" => Some("CLAUDE_CONFIG_DIR"),
        "codex" => Some("CODEX_HOME"),
        "muse" => Some("XDG_DATA_HOME"),
        _ => None,
    }
}

/// The per-host default configuration directory name under `$HOME`.
fn config_dir_name(host: &str) -> &'static str {
    match host {
        "codex" => ".codex",
        "cursor-agent" => ".cursor",
        "hermes" => ".hermes",
        _ => ".claude",
    }
}

/// Resolve the host configuration directory: an explicit override wins, then
/// the host's own environment variable, then `$HOME/<default>`.
///
/// Muse appends `muse` itself, so for that host an explicit override names
/// the `XDG_DATA_HOME` value rather than the exact directory.
///
/// # Errors
/// [`Error::Usage`] when neither an override nor `HOME` is available.
pub fn config_dir(host: &str, explicit: Option<PathBuf>) -> Result<PathBuf> {
    if host == "muse" {
        let base = explicit
            .or(crate::config::env::env_parse::<PathBuf>("XDG_DATA_HOME")?)
            .or(crate::config::env::env_parse::<PathBuf>("HOME")?
                .map(|home| home.join(".local/share")))
            .ok_or_else(|| Error::Usage("HOME is unset; pass --config-dir".into()))?;
        return Ok(std::path::absolute(base.join("muse"))?);
    }
    let configured = explicit.or(match config_env_key(host) {
        Some(key) => crate::config::env::env_parse::<PathBuf>(key)?,
        None => None,
    });
    let root = if let Some(root) = configured {
        root
    } else {
        crate::config::env::env_parse::<PathBuf>("HOME")?
            .ok_or_else(|| Error::Usage("HOME is unset; pass --config-dir".into()))?
            .join(config_dir_name(host))
    };
    Ok(std::path::absolute(root)?)
}

/// The environment scoping for one host CLI invocation: the variable to set
/// and the value scoping it at `config_dir`. Muse owns the `muse` leaf, so
/// the child sees its parent as `XDG_DATA_HOME`; every other CLI host is
/// scoped at the config dir itself. File-based hosts never invoke a CLI, so
/// they have no scoping.
fn scope_env(host: &str, config_dir: &Path) -> Option<(&'static str, PathBuf)> {
    let key = config_env_key(host)?;
    let value = if host == "muse" {
        config_dir.parent()?.to_path_buf()
    } else {
        config_dir.to_path_buf()
    };
    Some((key, value))
}

/// Extract the embedded integration and register it with the native host CLI.
/// The bundle lands under the `Ctx`'s own data directory, so a test driving a
/// temporary `Paths` never touches `$HOME/.comemory`.
///
/// # Errors
/// [`Error::Usage`] for an unknown host or an unusable path;
/// [`Error::Unavailable`] when the host CLI or a hook dependency is missing or
/// one of the host's own plugin commands fails.
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Response> {
    let host = validate_host(&req.host)?;
    let root = std::path::absolute(
        ctx.paths
            .data_dir()
            .join("integrations")
            .join(env!("CARGO_PKG_VERSION")),
    )?;
    let config_dir = config_dir(host, req.config_dir)?;
    let mcp_manifest = if req.dry_run {
        root.join("plugins/comemory").join(".mcp.json")
    } else {
        let manifest = install(host, &config_dir, &root)?;
        // Written only after the host's own CLI accepted the plugin, so the
        // marker never claims an install that did not finish.
        std::fs::write(
            marker_path(ctx.paths.data_dir(), host),
            env!("CARGO_PKG_VERSION"),
        )?;
        manifest
    };
    Ok(Response {
        marketplace: root.parent().map(Path::to_path_buf),
        host: host.to_string(),
        plugin: PLUGIN,
        bundle: root,
        mcp_manifest,
        config_dir,
        dry_run: req.dry_run,
        installed: !req.dry_run,
    })
}

/// Probe the hook dependencies, extract the bundle, then register it through
/// the host's own mechanism: the native plugin CLI where one accepts a local
/// bundle, a file-based install otherwise. Returns the `.mcp.json` path so
/// the caller's report matches what was written.
fn install(host: &str, config_dir: &Path, root: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(config_dir)?;
    if matches!(host, "claude" | "codex" | "muse") {
        run_host(host, Some(config_dir), &["--version"])?;
    }
    for dependency in HOOK_DEPENDENCIES {
        let status = Command::new(dependency).arg("--version").output()?;
        if !status.status.success() {
            return Err(Error::Unavailable(format!(
                "{dependency} is required for agent hooks"
            )));
        }
    }
    bundle::extract(root)?;
    let binary = std::fs::canonicalize(std::env::current_exe()?)?;
    let manifest = bundle::write_mcp_manifest(&root.join("plugins/comemory"), &binary)?;
    match host {
        "claude" | "codex" => install_cli_plugin(host, config_dir, root)?,
        "muse" => install_muse_plugin(host, config_dir, root)?,
        "cursor-agent" => cursor::install(config_dir, root)?,
        "hermes" => hermes::install(config_dir, root)?,
        _ => {
            return Err(Error::Usage(format!(
                "unknown agent host `{host}` (known: {})",
                HOSTS.join(", ")
            )));
        }
    }
    Ok(manifest)
}

/// Register through the Claude/Codex `plugin` CLI: marketplace first, then
/// the plugin itself. `run` validated the host, so the dispatch above is the
/// only caller.
fn install_cli_plugin(host: &str, config_dir: &Path, root: &Path) -> Result<()> {
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
    let action = if host == "codex" { "add" } else { "install" };
    run_host(host, Some(config_dir), &["plugin", action, PLUGIN])?;
    if host == "claude" {
        run_host(host, Some(config_dir), &["plugin", "update", PLUGIN])?;
    }
    Ok(())
}

/// Register through the muse `plugins` CLI by direct path install: no
/// marketplace catalog is needed, and installs already report enabled
/// trust, so no approve step follows.
fn install_muse_plugin(host: &str, config_dir: &Path, root: &Path) -> Result<()> {
    let plugin = root.join("plugins/comemory");
    let path = plugin
        .to_str()
        .ok_or_else(|| Error::Usage("integration path must be UTF-8".into()))?;
    run_host(host, Some(config_dir), &["plugins", "install", path])?;
    Ok(())
}

/// Invoke the host's own CLI with `args`, scoped to `config_dir` through the
/// host's configuration environment variable.
fn run_host(host: &str, config_dir: Option<&Path>, args: &[&str]) -> Result<()> {
    let mut command = Command::new(host);
    command.args(args);
    if let Some(dir) = config_dir
        && let Some((key, value)) = scope_env(host, dir)
    {
        command.env(key, value);
    }
    let output = command.output().map_err(|error| {
        Error::Unavailable(format!(
            "{host} {}: {error}. Install the host CLI before retrying.",
            args.join(" ")
        ))
    })?;
    if !output.status.success() {
        return Err(Error::Unavailable(format!(
            "{host} {} failed ({}): {} {}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        )));
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/install.rs"]
mod tests;
