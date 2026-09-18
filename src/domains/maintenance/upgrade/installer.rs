//! Handing the actual binary swap to the code that already knows how to do
//! it: the release's own `install.sh` for a standalone binary, `brew` for a
//! Homebrew one. `comemory upgrade` decides *whether* and *to what*; this
//! module only runs the tool that does *how*.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::release;
use super::version::Version;
use crate::prelude::*;
use crate::store::random_id::random_hex;

/// The installer asset every release carries (uploaded by
/// `.github/workflows/release-finalize.yml` from the tag's own tree).
pub const SCRIPT_ASSET: &str = "install.sh";

/// Fetch `<base>/download/<tag>/install.sh` and run it pinned to `tag`,
/// into `dir`, with rc files untouched (the binary is already reachable —
/// that is how it is running). `quiet` captures the script's output instead
/// of letting it paint the terminal; a failure then carries the tail of what
/// it said.
pub fn run_script(base: &str, tag: &str, dir: &Path, quiet: bool) -> Result<()> {
    let url = format!("{base}/download/{tag}/{SCRIPT_ASSET}");
    let workdir = private_workdir()?;
    let script = workdir.join(SCRIPT_ASSET);
    let result = release::download(&url, &script).and_then(|()| {
        let mut cmd = Command::new("sh");
        cmd.arg(&script)
            .args(["--version", tag, "--dir"])
            .arg(dir)
            .arg("--no-modify-path");
        if quiet {
            cmd.arg("--quiet");
        }
        run_tool(&mut cmd, SCRIPT_ASSET, quiet)
    });
    if let Err(e) = std::fs::remove_dir_all(&workdir) {
        tracing::warn!(dir = %workdir.display(), error = %e, "could not remove the upgrade workdir");
    }
    result
}

/// A fresh, owner-only (`0700`) directory under the system temp dir to
/// download the script into. A predictable path in the shared temp dir
/// would let another local user pre-plant a symlink for `curl -o` to follow
/// (CWE-377); a 16-byte random name (128 bits, from `/dev/urandom`) plus
/// `create`-not-`create_dir_all` (fails if the name is taken) plus the mode
/// closes that. Unix-only mode bits — every published target is unix, and
/// `sh` is required here anyway.
fn private_workdir() -> Result<PathBuf> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    let dir = std::env::temp_dir().join(format!("comemory-upgrade-{}", random_hex(16)?));
    builder.create(&dir)?;
    Ok(dir)
}

/// `brew upgrade comemory`, with the same quiet/inherit split as
/// [`run_script`].
pub fn brew_upgrade(quiet: bool) -> Result<()> {
    let mut cmd = Command::new("brew");
    cmd.args(["upgrade", "comemory"]);
    run_tool(&mut cmd, "brew upgrade comemory", quiet)
}

/// Run `exe --version` and parse the `comemory X.Y.Z` it prints — the
/// proof, after a swap, that the file on disk is the release we asked for.
pub fn installed_version(exe: &Path) -> Result<Version> {
    let out = Command::new(exe)
        .arg("--version")
        .stdin(Stdio::null())
        .output()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let raw = text.split_whitespace().nth(1).ok_or_else(|| {
        Error::Other(format!(
            "{} --version printed `{}`, not `comemory X.Y.Z`",
            exe.display(),
            text.trim()
        ))
    })?;
    Version::parse(raw)
}

/// Run an installer tool to completion. `quiet` pipes stdout/stderr (the
/// `--json` path must own stdout) and folds the last lines of stderr into
/// the error; otherwise the tool inherits the terminal and paints its own
/// progress.
fn run_tool(cmd: &mut Command, what: &str, quiet: bool) -> Result<()> {
    cmd.stdin(Stdio::null());
    let spawn_failed = |e: std::io::Error| Error::Other(format!("{what}: {e}"));
    if !quiet {
        let status = cmd.status().map_err(spawn_failed)?;
        return if status.success() {
            Ok(())
        } else {
            Err(Error::Other(format!("{what} exited with {status}")))
        };
    }
    let out = cmd.output().map_err(spawn_failed)?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let mut tail: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .rev()
        .take(4)
        .collect();
    tail.reverse();
    Err(Error::Other(format!(
        "{what} exited with {}: {}",
        out.status,
        tail.join(" | ")
    )))
}
