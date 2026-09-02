#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Shared subprocess runner for crate-root CLI journey tests
//! (`tests/cli_scenario_*.rs`).
//!
//! Each consuming binary `#[path]`-includes this file directly, matching
//! `git_repo.rs`: a `pub` item pulled into a binary must be used there or
//! `-D warnings` fails. `COMEMORY_DATA_DIR` is `<temp>/.comemory` so the
//! layout matches production (`config.toml`, `comemory.db`, `memories/`).

use assert_cmd::Command;
use serde_json::Value;
use std::path::PathBuf;
use tempfile::TempDir;

/// A throwaway data directory plus a `comemory` binary pointed at it.
pub struct CliHome {
    root: TempDir,
}

impl CliHome {
    /// Fresh tempdir. The `.comemory` directory is created by the first
    /// command that calls `Paths::ensure_dirs`.
    pub fn new() -> Self {
        Self {
            root: TempDir::new().expect("tempdir"),
        }
    }

    /// `<root>/.comemory` — pass this as `COMEMORY_DATA_DIR`.
    pub fn data_dir(&self) -> PathBuf {
        self.root.path().join(".comemory")
    }

    /// `comemory` with `COMEMORY_DATA_DIR` set. Callers that need a `Path`
    /// argument chain `.arg(path)` on the returned command.
    pub fn bin(&self) -> Command {
        let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
        c.env("COMEMORY_DATA_DIR", self.data_dir());
        c
    }

    /// Run `comemory <args>` to success and return stdout.
    pub fn run_ok(&self, args: &[&str]) -> String {
        let out = self.bin().args(args).output().expect("run comemory");
        assert!(
            out.status.success(),
            "comemory {args:?} failed (status {:?}): {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).expect("stdout utf8")
    }

    /// Run `comemory --json <args>` to success and parse stdout as JSON.
    pub fn run_json(&self, args: &[&str]) -> Value {
        let stdout = self.run_ok(&prepend_json(args));
        serde_json::from_str(stdout.trim())
            .unwrap_or_else(|e| panic!("--json {args:?} is not JSON: {e}\n{stdout}"))
    }
}

fn prepend_json<'a>(args: &'a [&'a str]) -> Vec<&'a str> {
    let mut v = Vec::with_capacity(args.len() + 1);
    v.push("--json");
    v.extend_from_slice(args);
    v
}
