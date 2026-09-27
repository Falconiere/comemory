#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
#![cfg(unix)]
//! `install.sh` on its own, against the loopback stand-in for GitHub
//! Releases (`tests/common/release_server.rs`): the real script, run by
//! `/bin/sh` inside a `DaemonHome` (#258) — a private `HOME`, data
//! directory, and the `process` supervisor, so an install here can never
//! register a launchd agent in the developer's own session. Coordinators an
//! install starts are stopped when the fixture drops.

#[path = "common/daemon_support.rs"]
mod daemon_support;
#[path = "common/install_rig.rs"]
mod install_rig;
#[path = "common/release_server.rs"]
mod release_server;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;

use install_rig::{Rig, SCRIPT, current_tag};
use release_server::{host_target, stage_real_release};

const READY: Duration = Duration::from_secs(30);

/// `comemory <CARGO_PKG_VERSION>`, the real branch binary's own `--version`
/// line — every install here places this same binary, whatever tag names
/// it.
fn real_version() -> String {
    format!("comemory {}", env!("CARGO_PKG_VERSION"))
}

/// A rig with the real branch binary staged as `current_tag()` (`latest`);
/// `stub_tags` add `--version`-only stub releases alongside it.
struct Fixture {
    rig: Rig,
}

impl Fixture {
    fn new(stub_tags: &[&str]) -> Self {
        Self {
            rig: Rig::new(stub_tags),
        }
    }

    /// `<root>/releases`, the directory the loopback server publishes from.
    fn root(&self) -> &Path {
        &self.rig.releases
    }

    /// `<root>/<name>`, a directory under the rig's private root (never
    /// `HOME`) a test names for an install target.
    fn dir(&self, name: &str) -> PathBuf {
        self.rig.dir(name)
    }

    /// The private `HOME` install.sh sees; shell rc files land here.
    fn home_dir(&self) -> PathBuf {
        self.rig.home.home_dir()
    }

    /// `sh install.sh <args>` in the rig's daemon-safe environment.
    /// `extra_env` is applied last so a test can add `PATH`, `SHELL`, or
    /// `COMEMORY_*` overrides.
    fn run(&self, args: &[&str], extra_env: &[(&str, &str)]) -> Output {
        let mut cmd = self.rig.home.command_with_binary(Path::new("sh"));
        cmd.arg(SCRIPT)
            .args(args)
            .env("COMEMORY_RELEASES_URL", &self.rig.srv.base)
            .env("NO_COLOR", "1")
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("XDG_DATA_HOME")
            .env_remove("ZDOTDIR")
            .env_remove("COMEMORY_INSTALL_DIR")
            .env_remove("COMEMORY_VERSION")
            .env_remove("COMEMORY_NO_MODIFY_PATH");
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        cmd.output().expect("run install.sh")
    }
}

fn version_of(bin: &Path) -> String {
    let out = Command::new(bin).arg("--version").output().unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn ok(out: &Output) -> String {
    assert!(
        out.status.success(),
        "install.sh failed ({:?}):\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn installs_latest_into_dir_and_reports_each_step() {
    let fx = Fixture::new(&[]);
    let dir = fx.dir("bin");
    let stdout = ok(&fx.run(&["--dir", dir.to_str().unwrap(), "--no-modify-path"], &[]));
    let ready = fx.rig.home.wait_ready(READY);
    for expected in [
        "Platform".to_string(),
        "Version".to_string(),
        format!("{} (latest)", current_tag()),
        "Install dir".to_string(),
        "Checksum".to_string(),
        "sha256".to_string(),
        "Installed".to_string(),
        format!("comemory {} installed", current_tag()),
        format!("Sync daemon  ready ({}, pid {}", ready.version, ready.pid),
        "comemory doctor".to_string(),
        "comemory upgrade".to_string(),
    ] {
        assert!(
            stdout.contains(&expected),
            "missing {expected:?} in:\n{stdout}"
        );
    }
    assert_eq!(version_of(&dir.join("comemory")), real_version());
}

#[test]
fn env_pins_version_and_dir() {
    let fx = Fixture::new(&[]);
    let target = host_target().expect("comemory publishes a build for this host");
    stage_real_release(fx.root(), "v1.2.3", target);
    let dir = fx.dir("elsewhere");
    let stdout = ok(&fx.run(
        &["--no-modify-path", "--quiet"],
        &[
            ("COMEMORY_VERSION", "1.2.3"),
            ("COMEMORY_INSTALL_DIR", dir.to_str().unwrap()),
        ],
    ));
    assert!(
        stdout.is_empty(),
        "--quiet prints nothing on success: {stdout}"
    );
    assert_eq!(version_of(&dir.join("comemory")), real_version());
}

#[test]
fn rerun_replaces_the_comemory_already_on_path() {
    let fx = Fixture::new(&[]);
    let target = host_target().expect("comemory publishes a build for this host");
    stage_real_release(fx.root(), "v1.2.3", target);
    let dir = fx.dir("bin");
    ok(&fx.run(
        &[
            "--version",
            "v1.2.3",
            "--dir",
            dir.to_str().unwrap(),
            "--no-modify-path",
        ],
        &[],
    ));
    assert_eq!(version_of(&dir.join("comemory")), real_version());
    let path = format!("{}:/usr/bin:/bin", dir.display());
    let stdout = ok(&fx.run(&["--no-modify-path"], &[("PATH", &path)]));
    assert!(
        stdout.contains(&format!("replacing {}", dir.join("comemory").display())),
        "{stdout}"
    );
    assert_eq!(version_of(&dir.join("comemory")), real_version());
}

#[test]
fn without_dir_it_falls_through_to_an_existing_cargo_bin() {
    let fx = Fixture::new(&[]);
    let cargo_home = fx.dir("cargo");
    std::fs::create_dir_all(cargo_home.join("bin")).unwrap();
    let stdout = ok(&fx.run(
        &["--no-modify-path"],
        &[
            ("CARGO_HOME", cargo_home.to_str().unwrap()),
            ("PATH", "/usr/bin:/bin"),
        ],
    ));
    assert!(stdout.contains("(cargo bin directory)"), "{stdout}");
    assert_eq!(
        version_of(&cargo_home.join("bin").join("comemory")),
        real_version()
    );
    assert!(
        !fx.home_dir().join(".local/bin/comemory").exists(),
        "the binary did not fall through to the last-resort install dir"
    );
}

#[test]
fn without_dir_or_cargo_bin_it_uses_local_bin() {
    let fx = Fixture::new(&[]);
    let cargo_home = fx.dir("no-such-cargo");
    let stdout = ok(&fx.run(
        &["--no-modify-path"],
        &[
            ("PATH", "/usr/bin:/bin"),
            ("CARGO_HOME", cargo_home.to_str().unwrap()),
        ],
    ));
    assert!(stdout.contains("(default)"), "{stdout}");
    let bin = fx.home_dir().join(".local").join("bin").join("comemory");
    assert_eq!(version_of(&bin), real_version());
}

#[test]
fn checksum_mismatch_aborts_and_installs_nothing() {
    let fx = Fixture::new(&[]);
    let target = host_target().expect("comemory publishes a build for this host");
    let sidecar = fx
        .root()
        .join(current_tag())
        .join(format!("comemory-{target}.tar.xz.sha256"));
    std::fs::write(&sidecar, format!("{} *x\n", "f".repeat(64))).unwrap();
    let dir = fx.dir("bin");
    let out = fx.run(&["--dir", dir.to_str().unwrap(), "--no-modify-path"], &[]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("checksum mismatch"), "{stderr}");
    assert!(stderr.contains("nothing was installed"), "{stderr}");
    assert!(!dir.join("comemory").exists());
}

#[test]
fn missing_release_names_the_url() {
    let fx = Fixture::new(&[]);
    let dir = fx.dir("bin");
    let out = fx.run(
        &[
            "--version",
            "v4.4.4",
            "--dir",
            dir.to_str().unwrap(),
            "--no-modify-path",
        ],
        &[],
    );
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("download failed"), "{stderr}");
    assert!(stderr.contains("/download/v4.4.4/"), "{stderr}");
}

#[test]
fn help_and_unknown_option_exit_codes() {
    let fx = Fixture::new(&[]);
    let help = fx.run(&["--help"], &[]);
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("--no-modify-path"));
    let bogus = fx.run(&["--bogus"], &[]);
    assert_eq!(bogus.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&bogus.stderr).contains("unknown option: --bogus"));
}

/// TEXT check: no supported CI host reaches `detect_target`'s unsupported
/// branch without faking `uname`, so this reads install.sh's source instead
/// of running it.
#[test]
fn unsupported_platform_hints_name_the_wrapper() {
    let text = std::fs::read_to_string(SCRIPT).unwrap();
    let hints: Vec<&str> = text
        .lines()
        .filter(|l| l.contains("build from source"))
        .collect();
    assert!(hints.len() >= 2, "{text}");
    for hint in &hints {
        assert!(hint.contains("bash scripts/dev-install.sh"), "{hint}");
    }
    assert!(!text.contains("cargo install --path ."), "{text}");
}
