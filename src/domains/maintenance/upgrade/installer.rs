//! Handing the actual binary swap to the code that already knows how to do
//! it: the release's own `install.sh` for a standalone binary, `brew` for a
//! Homebrew one. `comemory upgrade` decides *whether* and *to what*; this
//! module only runs the tool that does *how*.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Serialize;

use super::release;
use super::version::Version;
use crate::prelude::*;
use crate::store::random_id::random_hex;

/// The installer asset every release carries (uploaded by
/// `.github/workflows/release-finalize.yml` from the tag's own tree).
pub const SCRIPT_ASSET: &str = "install.sh";

/// Fetch `<base>/download/<tag>/install.sh` and run it pinned to `tag`,
/// into `dir`, without adding another PATH startup-file entry (the binary is
/// already reachable — that is how it is running). Completion registrations
/// are refreshed by the installer, and it ensures the sync daemon of
/// `data_dir` on the new binary (rolling back and exiting 69 when it cannot).
/// `quiet` captures the script's output instead of letting it paint the
/// terminal; a failure then carries the tail of what it said.
pub fn run_script(base: &str, tag: &str, dir: &Path, data_dir: &Path, quiet: bool) -> Result<()> {
    let url = format!("{base}/download/{tag}/{SCRIPT_ASSET}");
    let workdir = private_workdir()?;
    let script = workdir.join(SCRIPT_ASSET);
    let result = release::download(&url, &script).and_then(|()| {
        let mut cmd = Command::new("sh");
        cmd.arg(&script)
            .args(["--version", tag, "--dir"])
            .arg(dir)
            .arg("--no-modify-path")
            .env("COMEMORY_DATA_DIR", data_dir);
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
/// progress. Exit 69 (`install.sh`: the sync daemon never became ready, the
/// previous binary restored) stays [`Error::Unavailable`].
fn run_tool(cmd: &mut Command, what: &str, quiet: bool) -> Result<()> {
    cmd.stdin(Stdio::null());
    let spawn_failed = |e: std::io::Error| Error::Other(format!("{what}: {e}"));
    let failed = |status: std::process::ExitStatus, detail: String| {
        let message = format!("{what} exited with {status}{detail}");
        if status.code() == Some(EX_UNAVAILABLE) {
            Error::Unavailable(message)
        } else {
            Error::Other(message)
        }
    };
    if !quiet {
        let status = cmd.status().map_err(spawn_failed)?;
        return if status.success() {
            Ok(())
        } else {
            Err(failed(status, String::new()))
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
    Err(failed(out.status, format!(": {}", tail.join(" | "))))
}

/// `install.sh`'s and `sync daemon ensure`'s "service unavailable" code.
const EX_UNAVAILABLE: i32 = 69;

/// The coordinator `comemory upgrade` verified after its swap (or its
/// no-op): what `sync daemon ensure --json` reported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DaemonReport {
    /// Always `true` in a report; a coordinator that is not ready fails.
    pub ready: bool,
    /// What `ensure` did (`none`, `started`, `replaced`, …).
    pub action: String,
    /// The backend keeping it running.
    pub supervisor: String,
    /// The version it answers with.
    pub version: String,
    /// The canonical binary it runs.
    pub binary: PathBuf,
    /// `<dev>:<ino>` of that binary, when it reports one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary_file: Option<String>,
    /// Its pid.
    pub pid: u64,
    /// The data directory it serves.
    pub data_dir: PathBuf,
}

/// Run `<exe> --data-dir <data_dir> sync daemon ensure --json` — the
/// installed binary, never this (possibly replaced) process — and require a
/// ready coordinator on `expect` at `exe`.
///
/// # Errors
/// The cause, as text, when the child cannot run or the coordinator is not
/// the expected one; the caller words the user-facing error.
pub fn ensure_child(
    exe: &Path,
    data_dir: &Path,
    expect: &Version,
) -> std::result::Result<DaemonReport, String> {
    let out = Command::new(exe)
        .arg("--data-dir")
        .arg(data_dir)
        .args(["sync", "daemon", "ensure", "--json"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run {}: {e}", exe.display()))?;
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|_| {
        let stderr = String::from_utf8_lossy(&out.stderr);
        format!(
            "sync daemon ensure exited with {}: {}",
            out.status,
            stderr.trim()
        )
    })?;
    verify_ensured(&doc, exe, expect)
}

/// Check an `ensure --json` document against the binary it must name.
///
/// # Errors
/// Why the reported coordinator is not ready, or not `expect` at `exe`.
pub fn verify_ensured(
    doc: &serde_json::Value,
    exe: &Path,
    expect: &Version,
) -> std::result::Result<DaemonReport, String> {
    if doc["ready"] != true {
        return Err(doc["error"].as_str().unwrap_or("not ready").to_string());
    }
    let daemon = &doc["daemon"];
    let text = |key: &str| daemon[key].as_str().unwrap_or_default().to_string();
    let expected_path = std::fs::canonicalize(exe).unwrap_or_else(|_| exe.to_path_buf());
    let report = DaemonReport {
        ready: true,
        action: doc["action"].as_str().unwrap_or_default().to_string(),
        supervisor: doc["supervisor"].as_str().unwrap_or_default().to_string(),
        version: text("version"),
        binary: PathBuf::from(text("binary")),
        binary_file: daemon["binary_file"].as_str().map(str::to_string),
        pid: daemon["pid"].as_u64().unwrap_or_default(),
        data_dir: PathBuf::from(text("data_dir")),
    };
    if report.version != expect.to_string() || report.binary != expected_path {
        return Err(format!(
            "the coordinator answers as {} at {}, expected {expect} at {}",
            report.version,
            report.binary.display(),
            expected_path.display()
        ));
    }
    Ok(report)
}

/// The Homebrew-linked binary (`$(brew --prefix comemory)/bin/comemory`),
/// canonicalized to the Cellar file it points at after `brew upgrade`.
///
/// # Errors
/// `brew` cannot be run or names no installed comemory.
pub fn brew_binary() -> Result<PathBuf> {
    let out = Command::new("brew")
        .args(["--prefix", "comemory"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| Error::Other(format!("brew --prefix comemory: {e}")))?;
    if !out.status.success() {
        return Err(Error::Other(format!(
            "brew --prefix comemory exited with {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    let prefix = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let linked = PathBuf::from(prefix).join("bin/comemory");
    std::fs::canonicalize(&linked).map_err(|e| Error::Other(format!("{}: {e}", linked.display())))
}

#[cfg(test)]
#[path = "tests/installer.rs"]
mod tests;
