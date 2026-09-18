//! Which install channel put the running `comemory` where it is. `comemory
//! upgrade` only ever replaces a binary the shell installer (or a hand-unpacked
//! tarball) owns; Homebrew and `cargo install` builds have their own upgrade
//! paths, and clobbering them from here would leave the package manager's
//! bookkeeping pointing at a binary it did not write.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::config::env::env_parse;

/// How the running binary was installed, detected from its resolved path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Channel {
    /// Under a Homebrew Cellar / prefix: `brew upgrade comemory` owns it.
    Homebrew,
    /// Listed in `$CARGO_HOME/.crates.toml`: built by `cargo install` from
    /// the recorded `source` (a `path+file://…` checkout or a `git+…` URL).
    CargoInstall {
        /// The source `cargo install` recorded for it.
        source: String,
    },
    /// Anything else: the shell installer's binary (or a hand-unpacked
    /// tarball) in `dir`, which the installer can replace in place.
    Standalone {
        /// The directory holding the binary.
        dir: PathBuf,
    },
}

impl Channel {
    /// Short human label for the TTY report.
    pub fn label(&self) -> &'static str {
        match self {
            Channel::Homebrew => "homebrew",
            Channel::CargoInstall { .. } => "cargo install",
            Channel::Standalone { .. } => "standalone",
        }
    }
}

/// Classify `exe`, the running binary's path with symlinks already resolved
/// (Homebrew's `bin/comemory` is a symlink into the Cellar), against this
/// process's `$CARGO_HOME`.
pub fn detect(exe: &Path) -> Channel {
    detect_with(exe, cargo_home().as_deref())
}

/// [`detect`] with the cargo home passed in, so the `.crates.toml` branch
/// is testable without touching the process environment.
pub fn detect_with(exe: &Path, cargo_home: Option<&Path>) -> Channel {
    if is_homebrew(exe) {
        return Channel::Homebrew;
    }
    if let Some(source) = cargo_home.and_then(|home| cargo_install_source(exe, home)) {
        return Channel::CargoInstall { source };
    }
    let dir = exe
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    Channel::Standalone { dir }
}

/// A `Cellar` path component, or one of the two Homebrew prefixes that are
/// not spelled `Cellar` at the top (`/opt/homebrew` on Apple Silicon,
/// `/home/linuxbrew/.linuxbrew` on Linux).
fn is_homebrew(exe: &Path) -> bool {
    let text = exe.to_string_lossy();
    exe.components().any(|c| c.as_os_str() == "Cellar")
        || text.starts_with("/opt/homebrew/")
        || text.contains("/linuxbrew/")
}

/// `<cargo_home>/.crates.toml` records every `cargo install`ed crate as
/// `"comemory 0.18.2 (path+file:///src/comemory)" = ["comemory"]`. Returns
/// that source when the running binary sits in `<cargo_home>/bin` and the
/// file lists comemory.
fn cargo_install_source(exe: &Path, cargo_home: &Path) -> Option<String> {
    if exe.parent()? != cargo_home.join("bin") {
        return None;
    }
    let text = std::fs::read_to_string(cargo_home.join(".crates.toml")).ok()?;
    crates_toml_source(&text)
}

/// Pull the `(source)` of the `comemory` entry out of `.crates.toml` text;
/// `None` when the file lists no comemory.
pub fn crates_toml_source(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("\"comemory ")?;
        let (_, after) = rest.split_once('(')?;
        let (source, _) = after.split_once(')')?;
        Some(source.to_string())
    })
}

/// `$CARGO_HOME`, else `$HOME/.cargo`; `None` when neither is set.
fn cargo_home() -> Option<PathBuf> {
    if let Ok(Some(home)) = env_parse::<String>("CARGO_HOME") {
        return Some(PathBuf::from(home));
    }
    let home = env_parse::<String>("HOME").ok().flatten()?;
    Some(PathBuf::from(home).join(".cargo"))
}

#[cfg(test)]
#[path = "tests/channel.rs"]
mod tests;
