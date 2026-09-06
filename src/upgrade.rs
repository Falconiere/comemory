//! `comemory upgrade` — move the running binary to a newer release.
//!
//! The decision lives here; the mechanics are delegated. [`run`] resolves the
//! latest tag from GitHub Releases (`release`), works out how this binary was
//! installed (`channel`), compares versions (`version`), and then either
//! reports (`--check`) or hands the swap to the release's own `install.sh` /
//! to `brew` (`installer`). CLI-only: there is deliberately no `/api/v1`
//! route — a server replacing its own binary mid-request is not a feature
//! (`serve::routes::meta::CLI_ONLY`).
//!
//! No HTTP client is compiled in: fetches shell out to `curl`/`wget`, which
//! the installer needs anyway.

/// How the running binary was installed (Homebrew / `cargo install` /
/// standalone) and what that means for who may replace it.
pub mod channel;
/// Running `install.sh` or `brew` to perform the swap, and reading back the
/// installed version.
pub mod installer;
/// Resolving the latest tag and downloading assets via `curl`/`wget`.
pub mod release;
/// `MAJOR.MINOR.PATCH[-pre]` parsing and ordering.
pub mod version;

use std::path::PathBuf;

use serde::Serialize;

use crate::prelude::*;
use channel::Channel;
use version::Version;

/// What `comemory upgrade` was asked to do.
#[derive(Debug, Clone, Default)]
pub struct Request {
    /// Report only — never install.
    pub check: bool,
    /// Install this release instead of the latest (`0.19.0` or `v0.19.0`).
    pub version: Option<String>,
    /// Proceed when the target is not newer than the running build
    /// (reinstall or downgrade).
    pub force: bool,
    /// Keep the installer's output off the terminal (the `--json` path owns
    /// stdout).
    pub quiet: bool,
}

/// What happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// The running build already is the target.
    UpToDate,
    /// A newer release exists and `--check` stopped short of installing it.
    Available,
    /// The binary on disk is now the (newer) target.
    Upgraded,
    /// The binary on disk is now the target, which was not newer (`--force`
    /// reinstall or downgrade).
    Installed,
}

/// The `comemory upgrade` report, serialized verbatim under `--json`.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// The version that was running when the command started.
    pub current: String,
    /// The newest published release.
    pub latest: String,
    /// The release the command compared against / installed (`latest`
    /// unless `--version` pinned one).
    pub target: String,
    /// How the running binary was installed.
    pub channel: Channel,
    /// The binary's resolved path.
    pub exe: PathBuf,
    /// The outcome.
    pub status: Status,
    /// Advice when the command could not, or did not need to, act itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// The running build's version, from `CARGO_PKG_VERSION`.
pub fn current_version() -> Result<Version> {
    Version::parse(env!("CARGO_PKG_VERSION"))
}

/// Resolve, compare, and (unless `req.check`) install. See the module doc.
pub fn run(req: &Request) -> Result<Report> {
    let (current, target, mut report) = resolve(req)?;
    let newer = target > current;
    if req.check {
        report.status = if newer {
            Status::Available
        } else {
            Status::UpToDate
        };
        report.hint = newer.then(|| act_hint(&report.channel, &target));
        return Ok(report);
    }
    if !newer && !req.force {
        if target == current {
            return Ok(report);
        }
        return Err(Error::Usage(format!(
            "{target} is older than the running {current}; pass --force to downgrade"
        )));
    }
    swap(req, &report.channel, &target)?;
    verify(&mut report, &current, &target)?;
    Ok(report)
}

/// The running, target, and (provisionally `UpToDate`) report: one round
/// trip to resolve `latest`, one `current_exe` lookup to classify the
/// channel.
fn resolve(req: &Request) -> Result<(Version, Version, Report)> {
    let current = current_version()?;
    let exe = std::fs::canonicalize(std::env::current_exe()?)?;
    let latest = Version::parse(&release::latest_tag(&release::releases_url()?)?)?;
    let target = match &req.version {
        Some(v) => Version::parse(v)?,
        None => latest.clone(),
    };
    let report = Report {
        current: current.to_string(),
        latest: latest.to_string(),
        target: target.to_string(),
        channel: channel::detect(&exe),
        exe,
        status: Status::UpToDate,
        hint: None,
    };
    Ok((current, target, report))
}

/// Hand the swap to the channel's own tool. A `cargo install` build is
/// rebuilt from source, never swapped: that is an error carrying the recipe.
fn swap(req: &Request, channel: &Channel, target: &Version) -> Result<()> {
    match channel {
        Channel::Homebrew => {
            if req.version.is_some() {
                return Err(Error::Unsupported(
                    "Homebrew owns this binary and installs whatever the tap serves; \
                     `comemory upgrade` cannot pin a version there"
                        .into(),
                ));
            }
            installer::brew_upgrade(req.quiet)
        }
        Channel::CargoInstall { source } => Err(Error::Unsupported(cargo_hint(source, target))),
        Channel::Standalone { dir } => {
            installer::run_script(&release::releases_url()?, &target.tag(), dir, req.quiet)
        }
    }
}

/// Read the installed version back off disk and settle the status. A
/// Homebrew tap can lag a GitHub release by minutes; `brew upgrade` then
/// leaves `current` in place, which is reported as a hint, not a failure.
fn verify(report: &mut Report, current: &Version, target: &Version) -> Result<()> {
    let installed = installer::installed_version(&report.exe)?;
    if installed != *target {
        if report.channel == Channel::Homebrew && installed == *current {
            report.hint = Some(format!(
                "the Homebrew tap still serves {installed}; {target} has not reached it yet"
            ));
            return Ok(());
        }
        return Err(Error::Other(format!(
            "{} reports {installed} after the install, expected {target}",
            report.exe.display()
        )));
    }
    report.status = if installed > *current {
        Status::Upgraded
    } else {
        Status::Installed
    };
    Ok(())
}

/// What to run to get `target` onto this channel — the `--check` hint.
fn act_hint(channel: &Channel, target: &Version) -> String {
    match channel {
        Channel::Homebrew => "run: brew upgrade comemory".to_string(),
        Channel::CargoInstall { source } => cargo_hint(source, target),
        Channel::Standalone { .. } => "run: comemory upgrade".to_string(),
    }
}

/// A `cargo install` build is rebuilt from source, not swapped: name the
/// recorded source and the two ways to rebuild it at `target`.
fn cargo_hint(source: &str, target: &Version) -> String {
    format!(
        "this comemory was built by `cargo install` from {source}; rebuild it with \
         `cargo install --git https://github.com/Falconiere/comemory --tag {}` \
         (or `git pull && cargo install --path .` in that checkout)",
        target.tag()
    )
}
