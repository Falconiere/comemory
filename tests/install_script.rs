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
//! `/bin/sh`, downloading a real tarball and checksum from a real socket.
//! `HOME` is a temp dir so the PATH step can only ever write there.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use release_server::{ReleaseServer, host_target, stage_release, tooling_present};
use tempfile::TempDir;

#[path = "common/release_server.rs"]
mod release_server;

const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/install.sh");

struct Fixture {
    home: TempDir,
    srv: ReleaseServer,
}

impl Fixture {
    /// Serve `tags` (the first is `latest`) from a fresh temp root.
    fn new(tags: &[&str]) -> Option<Self> {
        let target = host_target()?;
        if !tooling_present() {
            return None;
        }
        let home = TempDir::new().unwrap();
        let root = home.path().join("releases");
        for tag in tags {
            stage_release(&root, tag, target);
        }
        let srv = ReleaseServer::start(root, tags[0]);
        Some(Self { home, srv })
    }

    fn root(&self) -> PathBuf {
        self.home.path().join("releases")
    }

    /// `sh install.sh <args>` with the fixture env. `extra_env` is applied
    /// last so a test can add `PATH`, `SHELL`, or `COMEMORY_*` overrides.
    fn run(&self, args: &[&str], extra_env: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new("sh");
        cmd.arg(SCRIPT)
            .args(args)
            .env("COMEMORY_RELEASES_URL", &self.srv.base)
            .env("HOME", self.home.path())
            .env("NO_COLOR", "1")
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
    let Some(fx) = Fixture::new(&["v9.9.9"]) else {
        return;
    };
    let dir = fx.home.path().join("bin");
    let stdout = ok(&fx.run(&["--dir", dir.to_str().unwrap(), "--no-modify-path"], &[]));
    for expected in [
        "Platform",
        "Version",
        "v9.9.9 (latest)",
        "Install dir",
        "Checksum",
        "sha256",
        "Installed",
        "comemory v9.9.9 installed",
        "comemory doctor",
        "comemory upgrade",
    ] {
        assert!(
            stdout.contains(expected),
            "missing {expected:?} in:\n{stdout}"
        );
    }
    assert_eq!(version_of(&dir.join("comemory")), "comemory 9.9.9");
}

#[test]
fn env_pins_version_and_dir() {
    let Some(fx) = Fixture::new(&["v9.9.9", "v1.2.3"]) else {
        return;
    };
    let dir = fx.home.path().join("elsewhere");
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
    assert_eq!(version_of(&dir.join("comemory")), "comemory 1.2.3");
}

#[test]
fn rerun_replaces_the_comemory_already_on_path() {
    let Some(fx) = Fixture::new(&["v9.9.9", "v1.2.3"]) else {
        return;
    };
    let dir = fx.home.path().join("bin");
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
    assert_eq!(version_of(&dir.join("comemory")), "comemory 1.2.3");
    let path = format!("{}:/usr/bin:/bin", dir.display());
    let stdout = ok(&fx.run(&["--no-modify-path"], &[("PATH", &path)]));
    assert!(
        stdout.contains(&format!("replacing {}", dir.join("comemory").display())),
        "{stdout}"
    );
    assert_eq!(version_of(&dir.join("comemory")), "comemory 9.9.9");
}

#[test]
fn without_dir_it_falls_through_to_an_existing_cargo_bin() {
    let Some(fx) = Fixture::new(&["v9.9.9"]) else {
        return;
    };
    let cargo_home = fx.home.path().join("cargo");
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
        "comemory 9.9.9"
    );
    assert!(
        !fx.home.path().join(".local").exists(),
        "not the last resort"
    );
}

#[test]
fn without_dir_or_cargo_bin_it_uses_local_bin() {
    let Some(fx) = Fixture::new(&["v9.9.9"]) else {
        return;
    };
    let mut cmd_env = vec![("PATH", "/usr/bin:/bin")];
    let cargo_home = fx.home.path().join("no-such-cargo");
    cmd_env.push(("CARGO_HOME", cargo_home.to_str().unwrap()));
    let stdout = ok(&fx.run(&["--no-modify-path"], &cmd_env));
    assert!(stdout.contains("(default)"), "{stdout}");
    let bin = fx.home.path().join(".local").join("bin").join("comemory");
    assert_eq!(version_of(&bin), "comemory 9.9.9");
}

#[test]
fn checksum_mismatch_aborts_and_installs_nothing() {
    let Some(fx) = Fixture::new(&["v9.9.9"]) else {
        return;
    };
    let sidecar = fx
        .root()
        .join("v9.9.9")
        .join(format!("comemory-{}.tar.xz.sha256", host_target().unwrap()));
    std::fs::write(&sidecar, format!("{} *x\n", "f".repeat(64))).unwrap();
    let dir = fx.home.path().join("bin");
    let out = fx.run(&["--dir", dir.to_str().unwrap(), "--no-modify-path"], &[]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("checksum mismatch"), "{stderr}");
    assert!(stderr.contains("nothing was installed"), "{stderr}");
    assert!(!dir.join("comemory").exists());
}

#[test]
fn missing_release_names_the_url() {
    let Some(fx) = Fixture::new(&["v9.9.9"]) else {
        return;
    };
    let dir = fx.home.path().join("bin");
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
fn path_line_is_appended_to_the_shell_rc_once() {
    let Some(fx) = Fixture::new(&["v9.9.9"]) else {
        return;
    };
    let dir = fx.home.path().join("bin");
    let rc = if cfg!(target_os = "macos") {
        fx.home.path().join(".bash_profile")
    } else {
        fx.home.path().join(".bashrc")
    };
    let env = [("SHELL", "/bin/bash"), ("PATH", "/usr/bin:/bin")];
    let first = ok(&fx.run(&["--dir", dir.to_str().unwrap()], &env));
    assert!(first.contains("added $HOME/bin to"), "{first}");
    let second = ok(&fx.run(&["--dir", dir.to_str().unwrap()], &env));
    assert!(second.contains("already adds $HOME/bin"), "{second}");
    let text = std::fs::read_to_string(&rc).unwrap();
    let line = "export PATH=\"$HOME/bin:$PATH\"";
    assert_eq!(text.matches(line).count(), 1, "{text}");
    assert!(text.contains("# added by the comemory installer"));
    assert!(first.contains("open a new shell"), "{first}");
}

#[test]
fn no_modify_path_leaves_rc_files_alone_and_says_so() {
    let Some(fx) = Fixture::new(&["v9.9.9"]) else {
        return;
    };
    let dir = fx.home.path().join("bin");
    let stdout = ok(&fx.run(
        &["--dir", dir.to_str().unwrap(), "--no-modify-path"],
        &[("SHELL", "/bin/bash"), ("PATH", "/usr/bin:/bin")],
    ));
    assert!(stdout.contains("rc files left alone"), "{stdout}");
    assert!(!fx.home.path().join(".bashrc").exists());
    assert!(!fx.home.path().join(".bash_profile").exists());
}

#[test]
fn help_and_unknown_option_exit_codes() {
    let Some(fx) = Fixture::new(&["v9.9.9"]) else {
        return;
    };
    let help = fx.run(&["--help"], &[]);
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("--no-modify-path"));
    let bogus = fx.run(&["--bogus"], &[]);
    assert_eq!(bogus.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&bogus.stderr).contains("unknown option: --bogus"));
}
