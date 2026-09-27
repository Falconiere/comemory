#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines,
    dead_code
)]
//! The real `install.sh` against the loopback release server, run inside a
//! `DaemonHome` (#258): a private `HOME`, data directory and `TMPDIR`, the
//! `process` supervisor, and the real branch binary as the release payload.
//! Every coordinator an install starts is stopped when the home drops.
//!
//! Consumers `#[path]`-include this beside `daemon_support.rs` and
//! `release_server.rs`.

use std::path::{Path, PathBuf};
use std::process::Output;

use sha2::{Digest as _, Sha256};

use super::daemon_support::DaemonHome;
use super::release_server::{
    ReleaseServer, host_target, stage_real_release, stage_release, tooling_present,
};

/// The installer under test.
pub const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/install.sh");

/// `v<CARGO_PKG_VERSION>`: the tag the real binary is staged under.
pub fn current_tag() -> String {
    format!("v{}", env!("CARGO_PKG_VERSION"))
}

/// A home, a release root, and a server publishing it.
pub struct Rig {
    /// The private home every command runs in.
    pub home: DaemonHome,
    /// `<tag>/<asset>` files the server serves.
    pub releases: PathBuf,
    /// The loopback server.
    pub srv: ReleaseServer,
}

impl Rig {
    /// The real binary as `latest` (`v<current>`), plus `stubs`: releases
    /// whose binary answers only `--version`, so it can never become ready.
    pub fn new(stubs: &[&str]) -> Self {
        let target = host_target().expect("comemory publishes a build for this host");
        assert!(tooling_present(), "tar, xz, curl and sh are required");
        let home = DaemonHome::new();
        let releases = home.root().join("releases");
        stage_real_release(&releases, &current_tag(), target);
        for tag in stubs {
            stage_release(&releases, tag, target);
        }
        let srv = ReleaseServer::start(releases.clone(), &current_tag());
        Self {
            home,
            releases,
            srv,
        }
    }

    /// `<root>/<name>`, the install directory a test names.
    pub fn dir(&self, name: &str) -> PathBuf {
        self.home.root().join(name)
    }

    /// `sh install.sh --dir <dir> --no-modify-path <args>` in the home's
    /// environment, `env` applied last.
    pub fn install(&self, dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
        let mut cmd = self.home.command_with_binary(Path::new("sh"));
        cmd.arg(SCRIPT)
            .arg("--dir")
            .arg(dir)
            .arg("--no-modify-path")
            .args(args)
            .env("COMEMORY_RELEASES_URL", &self.srv.base)
            .env("NO_COLOR", "1")
            .env("SHELL", "/bin/sh")
            .env_remove("COMEMORY_INSTALL_DIR")
            .env_remove("COMEMORY_VERSION")
            .env_remove("COMEMORY_NO_MODIFY_PATH");
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.output().expect("run install.sh")
    }

    /// [`Self::install`] asserting exit 0; returns stdout.
    pub fn install_ok(&self, dir: &Path, args: &[&str]) -> String {
        let out = self.install(dir, args, &[]);
        assert!(
            out.status.success(),
            "install.sh failed ({:?}):\n{}\n{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }
}

/// sha256 of the file at `path`.
pub fn sha256_of(path: &Path) -> Vec<u8> {
    Sha256::digest(std::fs::read(path).expect("read file")).to_vec()
}

/// `<dev>:<ino>` of the file at `path`.
pub fn file_id(path: &Path) -> String {
    use std::os::unix::fs::MetadataExt as _;
    let meta = std::fs::metadata(path).expect("stat");
    format!("{}:{}", meta.dev(), meta.ino())
}

/// `(exit code, stdout, stderr)` of an install.
pub fn parts(out: &Output) -> (i32, String, String) {
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}
