#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines,
    // Shared across several test binaries; each uses a different subset, so
    // unused-here is the normal case for a fixture.
    dead_code
)]
//! A throwaway `$HOME` + data dir for the platform-facing CLI suites.
//!
//! `comemory auth`, `comemory sync` and `comemory watch` all need the same
//! thing: an isolated data dir, `HOME` pointed at it, the daemon install
//! suppressed, and the ambient `COMEMORY_API*` vars cleared so a developer's
//! own credentials cannot leak into a test run. One copy, per D9.
//!
//! Each consuming binary `#[path]`-includes this file directly, matching
//! `cli_bin.rs` and `git_repo.rs`.

use std::path::PathBuf;
use std::process::Output;

use assert_cmd::cargo::cargo_bin;
use serde_json::Value;
use tempfile::TempDir;

/// An isolated `$HOME` whose `.comemory` is the data dir under test.
pub struct Home {
    root: TempDir,
}

impl Home {
    pub fn new() -> Self {
        Self {
            root: TempDir::new().unwrap(),
        }
    }

    pub fn data_dir(&self) -> PathBuf {
        self.root.path().join(".comemory")
    }

    pub fn auth_file(&self) -> PathBuf {
        self.data_dir().join("auth.json")
    }

    /// Run the real binary. `api` sets `COMEMORY_API`; `None` leaves it unset
    /// so the command must take its base URL from a flag or `auth.json`.
    pub fn run(&self, api: Option<&str>, args: &[&str]) -> Output {
        let mut cmd = std::process::Command::new(cargo_bin("comemory"));
        cmd.env("COMEMORY_DATA_DIR", self.data_dir())
            .env("HOME", self.root.path())
            .env("COMEMORY_SYNC_DAEMON", "0")
            .env_remove("COMEMORY_API")
            .env_remove("COMEMORY_API_KEY")
            .args(args);
        if let Some(url) = api {
            cmd.env("COMEMORY_API", url);
        }
        cmd.output().expect("run comemory")
    }

    /// Run with `--json` and parse stdout, asserting the command succeeded.
    pub fn run_json(&self, api: Option<&str>, args: &[&str]) -> Value {
        let mut full = vec!["--json"];
        full.extend_from_slice(args);
        let out = self.run(api, &full);
        assert!(
            out.status.success(),
            "expected success {:?}: {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
            panic!(
                "stdout not JSON: {e}\n{}",
                String::from_utf8_lossy(&out.stdout)
            )
        })
    }
}
