//! The per-host table behind `comemory install` and the `setup` agent-host
//! step: one [`Host`] row per agent host, so configuration-directory
//! resolution, the hook-dependency probe, the presence probe, the
//! registration dispatch and the report all read the same facts instead of
//! matching on host-name strings in each caller.

use std::path::{Path, PathBuf};

use crate::prelude::*;

/// How comemory registers its bundle with a host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mechanism {
    /// The Claude Code / Codex `plugin` CLI: marketplace add, then
    /// `plugin <install_verb>`, then `plugin update` when `refresh` is set.
    PluginCli {
        /// The subcommand that registers a plugin (`install` or `add`).
        install_verb: &'static str,
        /// Whether a separate `plugin update` refreshes an existing install.
        refresh: bool,
    },
    /// The muse `plugins` CLI: direct path install.
    MusePlugins,
    /// Cursor: skill links plus the `comemory` entry in `mcp.json`.
    CursorFiles,
    /// Hermes: skill links only.
    HermesFiles,
}

/// Where a host's configuration directory comes from when no explicit
/// `--config-dir` is given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigEnv {
    /// The variable names the configuration directory itself.
    Dir(&'static str),
    /// An XDG base-directory variable: the configuration directory is
    /// `<var>/<leaf>`. Per the XDG spec an empty or relative value counts as
    /// unset.
    XdgBase {
        /// The base-directory variable, e.g. `XDG_DATA_HOME`.
        var: &'static str,
        /// The host's own directory under that base, e.g. `muse`.
        leaf: &'static str,
    },
    /// No variable: `$HOME/<home_dir>`.
    Home,
}

/// How `setup` decides a host is available on this machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presence {
    /// The host binary is on `PATH` and answers `--version`.
    Cli,
    /// The host has no CLI to probe; its `$HOME/<home_dir>` exists.
    ConfigDir,
}

/// One agent host `comemory install` supports.
#[derive(Debug)]
pub struct Host {
    /// Canonical name, also the host binary `run_host` executes.
    pub name: &'static str,
    /// Where the configuration directory comes from.
    pub config_env: ConfigEnv,
    /// Default configuration directory relative to `$HOME`.
    pub home_dir: &'static str,
    /// How the bundle is registered.
    pub mechanism: Mechanism,
    /// How `setup` probes for the host.
    pub presence: Presence,
}

/// Every supported host, in the order `--help` and error messages list them.
pub const TABLE: &[Host] = &[
    Host {
        name: "claude",
        config_env: ConfigEnv::Dir("CLAUDE_CONFIG_DIR"),
        home_dir: ".claude",
        mechanism: Mechanism::PluginCli {
            install_verb: "install",
            refresh: true,
        },
        presence: Presence::Cli,
    },
    Host {
        name: "codex",
        config_env: ConfigEnv::Dir("CODEX_HOME"),
        home_dir: ".codex",
        mechanism: Mechanism::PluginCli {
            install_verb: "add",
            refresh: false,
        },
        presence: Presence::Cli,
    },
    Host {
        name: "muse",
        config_env: ConfigEnv::XdgBase {
            var: "XDG_DATA_HOME",
            leaf: "muse",
        },
        home_dir: ".local/share/muse",
        mechanism: Mechanism::MusePlugins,
        presence: Presence::Cli,
    },
    Host {
        name: "cursor-agent",
        config_env: ConfigEnv::Home,
        home_dir: ".cursor",
        mechanism: Mechanism::CursorFiles,
        presence: Presence::Cli,
    },
    Host {
        name: "hermes",
        config_env: ConfigEnv::Dir("HERMES_HOME"),
        home_dir: ".hermes",
        mechanism: Mechanism::HermesFiles,
        presence: Presence::ConfigDir,
    },
];

/// The plugin id every plugin-manager host registers the bundle under.
const PLUGIN: &str = "comemory@comemory";

/// Look `name` up in [`TABLE`].
///
/// # Errors
/// [`Error::Usage`] naming the offending value and every known host.
pub fn lookup(name: &str) -> Result<&'static Host> {
    TABLE.iter().find(|host| host.name == name).ok_or_else(|| {
        Error::Usage(format!(
            "unknown agent host `{name}` (known: {})",
            names().collect::<Vec<_>>().join(", ")
        ))
    })
}

/// Every host name, in [`TABLE`] order.
pub fn names() -> impl Iterator<Item = &'static str> {
    TABLE.iter().map(|host| host.name)
}

impl Host {
    /// Whether the install registers lifecycle hooks, and so needs the hook
    /// dependencies on `PATH`.
    pub fn installs_hooks(&self) -> bool {
        matches!(
            self.mechanism,
            Mechanism::PluginCli { .. } | Mechanism::MusePlugins
        )
    }

    /// External programs the install needs on `PATH`, probed before
    /// anything is written so a missing one fails the install rather than a
    /// later hook or skill wrapper. Every host's skill wrappers need bash,
    /// git and jq; the lifecycle hooks add python3.
    pub fn dependencies(&self) -> &'static [&'static str] {
        if self.installs_hooks() {
            &["bash", "git", "jq", "python3"]
        } else {
            &["bash", "git", "jq"]
        }
    }

    /// The plugin id the host's own manager records, for hosts that have one.
    pub fn plugin(&self) -> Option<&'static str> {
        self.installs_hooks().then_some(PLUGIN)
    }

    /// What the install delivers, for the human report.
    pub fn delivers(&self) -> &'static str {
        match self.mechanism {
            Mechanism::PluginCli { .. } | Mechanism::MusePlugins => "skills, hooks and MCP server",
            Mechanism::CursorFiles => "skills and MCP server",
            Mechanism::HermesFiles => "skills",
        }
    }

    /// What the operator still has to do after a successful install.
    pub fn next_steps(&self) -> Vec<String> {
        match self.mechanism {
            Mechanism::PluginCli { .. } => vec![
                format!("Restart {} to load the plugin.", self.name),
                format!(
                    "Migrating from toolu? Disable or uninstall comemory@toolu in {} to avoid duplicate hooks.",
                    self.name
                ),
            ],
            Mechanism::MusePlugins => vec![
                "Run `muse plugins approve comemory` to activate the hooks and MCP server; muse holds third-party hooks for review until then.".to_string(),
                "Restart muse to load the plugin.".to_string(),
            ],
            Mechanism::CursorFiles => {
                vec!["Restart Cursor to load the skills and MCP server.".to_string()]
            }
            Mechanism::HermesFiles => {
                vec!["Start a new hermes session to load the skills.".to_string()]
            }
        }
    }

    /// The environment scoping for one host CLI invocation: the variable to
    /// set and its value for `config_dir`. An XDG host owns its leaf, so the
    /// child sees the leaf's parent as the base; hosts without a variable
    /// get none.
    pub fn scope_env(&self, config_dir: &Path) -> Option<(&'static str, PathBuf)> {
        match self.config_env {
            ConfigEnv::Dir(key) => Some((key, config_dir.to_path_buf())),
            ConfigEnv::XdgBase { var, .. } => Some((var, config_dir.parent()?.to_path_buf())),
            ConfigEnv::Home => None,
        }
    }

    /// Resolve the configuration directory: an explicit override wins, then
    /// the host's environment variable, then `$HOME/<home_dir>`.
    ///
    /// An XDG host is scoped by its base variable, so an explicit override
    /// must end in the host's own leaf — anything else would install where
    /// the host never reads.
    ///
    /// # Errors
    /// [`Error::Usage`] when neither an override nor `HOME` is available, or
    /// when an XDG host's override does not end in its leaf.
    pub fn config_dir(&self, explicit: Option<PathBuf>) -> Result<PathBuf> {
        let home = crate::config::env::env_parse::<PathBuf>("HOME")?;
        self.config_dir_in(explicit, home)
    }

    /// [`Host::config_dir`] against an explicit `home`, so `setup` detection
    /// and its tests resolve exactly the directory an install would use.
    ///
    /// # Errors
    /// As [`Host::config_dir`].
    pub fn config_dir_in(
        &self,
        explicit: Option<PathBuf>,
        home: Option<PathBuf>,
    ) -> Result<PathBuf> {
        let configured = match (explicit, self.config_env) {
            (Some(dir), ConfigEnv::XdgBase { leaf, .. }) => {
                if dir.file_name().is_none_or(|name| name != leaf) {
                    return Err(Error::Usage(format!(
                        "--config-dir for {} must be its `{leaf}` directory (e.g. ~/{}), got {}",
                        self.name,
                        self.home_dir,
                        dir.display()
                    )));
                }
                Some(dir)
            }
            (Some(dir), _) => Some(dir),
            (None, ConfigEnv::Dir(key)) => non_empty_env(key)?,
            (None, ConfigEnv::XdgBase { var, leaf }) => non_empty_env(var)?
                .filter(|base| base.is_absolute())
                .map(|base| base.join(leaf)),
            (None, ConfigEnv::Home) => None,
        };
        let dir = match configured {
            Some(dir) => dir,
            None => home
                .filter(|home| !home.as_os_str().is_empty())
                .ok_or_else(|| Error::Usage("HOME is unset; pass --config-dir".into()))?
                .join(self.home_dir),
        };
        Ok(std::path::absolute(dir)?)
    }
}

/// Read `key` as a path, treating an empty value as unset.
fn non_empty_env(key: &str) -> Result<Option<PathBuf>> {
    Ok(crate::config::env::env_parse::<PathBuf>(key)?.filter(|dir| !dir.as_os_str().is_empty()))
}

/// Why no host is available, for `setup`'s unavailable agent-host step:
/// the one wanted host's own absence, or the full roster split by how each
/// is probed.
pub fn absence_reason(wanted: Option<&str>) -> String {
    let describe = |host: &Host| match host.presence {
        Presence::Cli => format!("no {} CLI found on PATH", host.name),
        Presence::ConfigDir => match host.config_env {
            ConfigEnv::Dir(key) => format!(
                "no {} config dir found (${key} or ~/{})",
                host.name, host.home_dir
            ),
            ConfigEnv::XdgBase { .. } | ConfigEnv::Home => {
                format!("no {} config dir found (~/{})", host.name, host.home_dir)
            }
        },
    };
    if let Some(host) = wanted.and_then(|name| TABLE.iter().find(|host| host.name == name)) {
        return describe(host);
    }
    let listed = |presence: Presence| {
        TABLE
            .iter()
            .filter(|host| host.presence == presence)
            .map(|host| host.name)
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "no supported agent host found: no {} CLI on PATH and no {} config dir",
        listed(Presence::Cli),
        listed(Presence::ConfigDir)
    )
}

#[cfg(test)]
#[path = "tests/host.rs"]
mod tests;
