//! Per-user installation for generated shell completion scripts.
//!
//! Bash, Zsh, and PowerShell need a startup-file registration to make a
//! per-user generated script reliably active; Fish autoloads its completion
//! directory. Managed package installers such as Homebrew use their own
//! prefix-specific completion directories instead of this module.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use clap_complete::Shell;
use serde::Serialize;

use crate::cli::completion_script;
use crate::prelude::*;

const BLOCK_START: &str = "# >>> comemory completions >>>";
const BLOCK_END: &str = "# <<< comemory completions <<<";

/// One completion script installed for a shell.
#[derive(Debug, Serialize)]
pub(super) struct InstalledCompletion {
    /// Shell understood by `comemory completions`.
    pub shell: &'static str,
    /// Destination written for that shell.
    pub path: PathBuf,
}

/// Result of installing all supported interactive-shell completions.
#[derive(Debug, Serialize)]
pub(super) struct Report {
    /// Generated completion scripts.
    pub installed: Vec<InstalledCompletion>,
    /// Startup profiles carrying one managed registration block.
    pub profiles: Vec<PathBuf>,
}

struct Roots {
    home: PathBuf,
    config: PathBuf,
    data: PathBuf,
    zdot: PathBuf,
}

/// Install Bash, Zsh, Fish, and PowerShell completions for the current user.
pub(super) fn run() -> Result<Report> {
    let roots = Roots::from_env()?;
    let bash = roots.data.join("bash-completion/completions/comemory");
    let zsh = roots.data.join("zsh/site-functions/_comemory");
    let fish = roots.config.join("fish/completions/comemory.fish");
    let powershell = roots.config.join("powershell/comemory.ps1");

    let installed = vec![
        install_script("bash", Shell::Bash, &bash)?,
        install_script("zsh", Shell::Zsh, &zsh)?,
        install_script("fish", Shell::Fish, &fish)?,
        install_script("powershell", Shell::PowerShell, &powershell)?,
    ];
    let profiles = register_profiles(&roots, &bash, &zsh, &powershell)?;
    Ok(Report {
        installed,
        profiles,
    })
}

impl Roots {
    fn from_env() -> Result<Self> {
        let home = env_path("HOME")
            .or_else(|| env_path("USERPROFILE"))
            .ok_or_else(|| Error::Other("cannot install completions: HOME is not set".into()))?;
        let config = env_path("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
        let data = env_path("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local/share"));
        let zdot = env_path("ZDOTDIR").unwrap_or_else(|| home.clone());
        Ok(Self {
            home,
            config,
            data,
            zdot,
        })
    }
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn install_script(name: &'static str, shell: Shell, path: &Path) -> Result<InstalledCompletion> {
    let script = completion_script::generate_for(shell)?;
    write_file(path, script.as_bytes())?;
    Ok(InstalledCompletion {
        shell: name,
        path: path.to_path_buf(),
    })
}

fn register_profiles(
    roots: &Roots,
    bash: &Path,
    zsh: &Path,
    powershell: &Path,
) -> Result<Vec<PathBuf>> {
    let bash_body = format!("[ -r {0} ] && . {0}", shell_quote(bash));
    let bashrc = roots.home.join(".bashrc");
    upsert_block(&bashrc, &bash_body)?;
    let (bash_login, created) = bash_login_profile(&roots.home);
    let bash_login_body = if created {
        format!(
            "[ -r {0} ] && . {0}\n{bash_body}",
            shell_quote(&roots.home.join(".profile"))
        )
    } else {
        bash_body
    };
    upsert_block(&bash_login, &bash_login_body)?;

    let zsh_body = format!(
        "fpath=({} $fpath)\nif (( ! $+functions[compdef] )); then\n  autoload -Uz compinit\n  compinit\nfi\n[ -r {1} ] && . {1}",
        shell_quote(zsh.parent().unwrap_or(Path::new("."))),
        shell_quote(zsh)
    );
    let zshrc = roots.zdot.join(".zshrc");
    upsert_block(&zshrc, &zsh_body)?;

    let ps_body = format!(". {}", powershell_quote(powershell));
    let ps_profile = powershell_profile(roots);
    upsert_block(&ps_profile, &ps_body)?;
    Ok(vec![bashrc, bash_login, zshrc, ps_profile])
}

fn bash_login_profile(home: &Path) -> (PathBuf, bool) {
    for name in [".bash_profile", ".bash_login", ".profile"] {
        let path = home.join(name);
        if path.exists() {
            return (path, false);
        }
    }
    (home.join(".bash_profile"), true)
}

#[cfg(not(windows))]
fn powershell_profile(roots: &Roots) -> PathBuf {
    roots.config.join("powershell/profile.ps1")
}

#[cfg(windows)]
fn powershell_profile(roots: &Roots) -> PathBuf {
    roots.home.join("Documents/PowerShell/Profile.ps1")
}

fn upsert_block(path: &Path, body: &str) -> Result<()> {
    let mut current = match std::fs::read_to_string(path) {
        Ok(value) => value,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    let block = format!("{BLOCK_START}\n{body}\n{BLOCK_END}");
    match (current.find(BLOCK_START), current.find(BLOCK_END)) {
        (Some(start), Some(end)) if end >= start => {
            current.replace_range(start..end + BLOCK_END.len(), &block);
        }
        (None, None) => {
            if !current.is_empty() && !current.ends_with('\n') {
                current.push('\n');
            }
            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(&block);
            current.push('\n');
        }
        _ => {
            return Err(Error::Other(format!(
                "incomplete comemory completion block in {}",
                path.display()
            )));
        }
    }
    write_file(path, current.as_bytes())
}

fn write_file(path: &Path, body: &[u8]) -> Result<()> {
    let target = resolve_write_target(path)?;
    let parent = target.parent().ok_or_else(|| {
        Error::Other(format!(
            "completion path has no parent: {}",
            target.display()
        ))
    })?;
    std::fs::create_dir_all(parent)?;
    let file_name = target.file_name().ok_or_else(|| {
        Error::Other(format!(
            "completion path has no file name: {}",
            target.display()
        ))
    })?;
    let permissions = match std::fs::metadata(&target) {
        Ok(metadata) => Some(metadata.permissions()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let temp = write_temp(parent, file_name, body, permissions.as_ref())?;
    #[cfg(windows)]
    if target.exists() {
        std::fs::remove_file(&target)?;
    }
    if let Err(e) = std::fs::rename(&temp, &target) {
        let _ = std::fs::remove_file(&temp);
        return Err(e.into());
    }
    Ok(())
}

fn resolve_write_target(path: &Path) -> Result<PathBuf> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Ok(std::fs::canonicalize(path)?),
        Ok(_) => Ok(path.to_path_buf()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(path.to_path_buf()),
        Err(e) => Err(e.into()),
    }
}

fn write_temp(
    parent: &Path,
    file_name: &std::ffi::OsStr,
    body: &[u8],
    permissions: Option<&std::fs::Permissions>,
) -> Result<PathBuf> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    for _ in 0..16 {
        let suffix = COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp = parent.join(format!(
            ".{}.tmp.{}.{}",
            file_name.to_string_lossy(),
            std::process::id(),
            suffix
        ));
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
        {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        };
        let written = file
            .write_all(body)
            .and_then(|()| permissions.map_or(Ok(()), |value| file.set_permissions(value.clone())));
        drop(file);
        if let Err(e) = written {
            let _ = std::fs::remove_file(&temp);
            return Err(e.into());
        }
        return Ok(temp);
    }
    Err(Error::Other(format!(
        "could not create a temporary completion file in {}",
        parent.display()
    )))
}

fn shell_quote(path: &Path) -> String {
    let value = path.to_string_lossy().replace('\'', "'\\''");
    format!("'{value}'")
}

fn powershell_quote(path: &Path) -> String {
    let value = path.to_string_lossy().replace('\'', "''");
    format!("'{value}'")
}
