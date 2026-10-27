//! `domains::integrations::install::{Request, Response, run}` — the shared middle of
//! `comemory install <host>`: extract the embedded agent integration, write
//! `<bundle>/plugins/comemory/.mcp.json` with this binary's absolute path,
//! and register it with the host — its native plugin manager where it has
//! one, a file-based install otherwise ([`host::TABLE`] decides). Moved out
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
pub(crate) mod host;
mod muse;
mod skill_links;

use host::{Host, Mechanism};

/// `comemory install` request. `host` is one of [`host::TABLE`]'s names.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Agent host name.
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
    /// What the install delivers to this host (skills, hooks, MCP server).
    pub delivers: &'static str,
    /// The plugin id registered with the host's plugin manager
    /// (`comemory@comemory`); `None` for file-based hosts.
    pub plugin: Option<&'static str>,
    /// Where the integration bundle was extracted.
    pub bundle: PathBuf,
    /// The host config file that names the `comemory mcp` server (written,
    /// or that would be written under `--dry-run`); `None` for a host
    /// without MCP support.
    pub mcp_manifest: Option<PathBuf>,
    /// The host configuration directory used.
    pub config_dir: PathBuf,
    /// Whether this was a preview.
    pub dry_run: bool,
    /// Whether files were actually written.
    pub installed: bool,
    /// What the operator still has to do for the host to load the install.
    pub next_steps: Vec<String>,
}

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
/// anything is written for a host that receives hooks, so a missing one
/// fails the install rather than surfacing later as a silently broken hook.
const HOOK_DEPENDENCIES: &[&str] = &["bash", "git", "jq", "python3"];

/// Validate `name` against [`host::TABLE`], returning the canonical
/// `&'static str` so downstream code never carries an unvalidated host
/// around.
///
/// # Errors
/// [`Error::Usage`] naming the offending value and the known hosts.
pub fn validate_host(name: &str) -> Result<&'static str> {
    host::lookup(name).map(|host| host.name)
}

/// Extract the embedded integration and register it with the host.
/// The bundle lands under the `Ctx`'s own data directory, so a test driving a
/// temporary `Paths` never touches `$HOME/.comemory`.
///
/// # Errors
/// [`Error::Usage`] for an unknown host or an unusable path;
/// [`Error::Unavailable`] when the host CLI or a hook dependency is missing or
/// one of the host's own plugin commands fails.
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Response> {
    let host = host::lookup(&req.host)?;
    let root = std::path::absolute(
        ctx.paths
            .data_dir()
            .join("integrations")
            .join(env!("CARGO_PKG_VERSION")),
    )?;
    let config_dir = host.config_dir(req.config_dir)?;
    if !req.dry_run {
        install(host, &config_dir, &root)?;
        // Written only after the host accepted the install, so the marker
        // never claims an install that did not finish.
        std::fs::write(
            marker_path(ctx.paths.data_dir(), host.name),
            env!("CARGO_PKG_VERSION"),
        )?;
    }
    Ok(Response {
        marketplace: root.parent().map(Path::to_path_buf),
        host: host.name.to_string(),
        delivers: host.delivers(),
        plugin: host.plugin(),
        mcp_manifest: mcp_manifest(host, &config_dir, &root),
        bundle: root,
        config_dir,
        dry_run: req.dry_run,
        installed: !req.dry_run,
        next_steps: host.next_steps(),
    })
}

/// The file that names the `comemory mcp` server for `host`.
fn mcp_manifest(host: &Host, config_dir: &Path, root: &Path) -> Option<PathBuf> {
    let plugin = root.join("plugins/comemory");
    match host.mechanism {
        Mechanism::PluginCli { .. } => Some(plugin.join(".mcp.json")),
        Mechanism::MusePlugins => Some(plugin.join(".muse-plugin/plugin.json")),
        Mechanism::CursorFiles => Some(config_dir.join("mcp.json")),
        Mechanism::HermesFiles => None,
    }
}

/// Probe what the host needs, extract the bundle, then register it through
/// the host's own mechanism: the native plugin CLI where one accepts a local
/// bundle, a file-based install otherwise.
fn install(host: &Host, config_dir: &Path, root: &Path) -> Result<()> {
    std::fs::create_dir_all(config_dir)?;
    if host.installs_hooks() {
        run_host(host, config_dir, &["--version"])?;
        for dependency in HOOK_DEPENDENCIES {
            let status = Command::new(dependency).arg("--version").output()?;
            if !status.status.success() {
                return Err(Error::Unavailable(format!(
                    "{dependency} is required for agent hooks"
                )));
            }
        }
    }
    bundle::extract(root)?;
    let binary = std::fs::canonicalize(std::env::current_exe()?)?;
    bundle::write_mcp_manifest(&root.join("plugins/comemory"), &binary)?;
    match host.mechanism {
        Mechanism::PluginCli {
            install_verb,
            refresh,
        } => install_cli_plugin(host, config_dir, root, install_verb, refresh),
        Mechanism::MusePlugins => muse::install(host, config_dir, root),
        Mechanism::CursorFiles => cursor::install(config_dir, root, &bundle::mcp_server(&binary)?),
        Mechanism::HermesFiles => skill_links::link_skills(root, &config_dir.join("skills")),
    }
}

/// Register through the Claude/Codex `plugin` CLI: marketplace first, then
/// the plugin itself, then a refresh where the host keeps a stale copy.
fn install_cli_plugin(
    host: &Host,
    config_dir: &Path,
    root: &Path,
    install_verb: &str,
    refresh: bool,
) -> Result<()> {
    let marketplace = root
        .parent()
        .ok_or_else(|| Error::Usage("bundle needs a parent".into()))?;
    let path = marketplace
        .to_str()
        .ok_or_else(|| Error::Usage("integration path must be UTF-8".into()))?;
    let plugin = host
        .plugin()
        .ok_or_else(|| Error::Usage(format!("{} has no plugin manager", host.name)))?;
    run_host(host, config_dir, &["plugin", "marketplace", "add", path])?;
    run_host(host, config_dir, &["plugin", install_verb, plugin])?;
    if refresh {
        run_host(host, config_dir, &["plugin", "update", plugin])?;
    }
    Ok(())
}

/// `path` canonicalized, or as given when it no longer exists — so an
/// ownership check holds however the data dir was spelled (`/var` vs
/// `/private/var`, a symlinked `$HOME`), and a vanished target still
/// compares by its recorded spelling.
fn canonical_or_raw(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Invoke the host's own CLI with `args`, scoped to `config_dir` through the
/// host's configuration environment variable, and return its stdout.
fn run_host(host: &Host, config_dir: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let mut command = Command::new(host.name);
    command.args(args);
    if let Some((key, value)) = host.scope_env(config_dir) {
        command.env(key, value);
    }
    let output = command.output().map_err(|error| {
        Error::Unavailable(format!(
            "{} {}: {error}. Install the host CLI before retrying.",
            host.name,
            args.join(" ")
        ))
    })?;
    if !output.status.success() {
        return Err(Error::Unavailable(format!(
            "{} {} failed ({}): {} {}",
            host.name,
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        )));
    }
    Ok(output.stdout)
}

#[cfg(test)]
#[path = "tests/install.rs"]
mod tests;
