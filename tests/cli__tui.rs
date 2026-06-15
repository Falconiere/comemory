//! Integration tests for `comemory tui`'s interactivity guard.
//!
//! The interactive loop cannot be driven under `assert_cmd` (no tty), but the
//! guard that rejects non-interactive invocations is fully testable: both
//! `--json` and a non-terminal render channel (stderr is piped here) must fail
//! cleanly with `EX_CONFIG` (78) before any terminal takeover.

use assert_cmd::Command;
use tempfile::TempDir;

/// Build a `comemory` invocation with `COMEMORY_DATA_DIR` rooted at `home`.
fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
}

#[test]
fn tui_rejects_json_with_ex_config() {
    let home = TempDir::new().expect("tempdir");
    let out = bin(&home).args(["tui", "--json"]).assert().code(78);
    let stderr = String::from_utf8(out.get_output().stderr.clone()).expect("utf8");
    assert!(
        stderr.contains("--json is not supported"),
        "expected json-rejection message, got: {stderr}"
    );
}

#[test]
fn tui_rejects_non_terminal_render_channel() {
    // Under assert_cmd stderr is a pipe, not a tty, so the interactivity guard
    // must refuse rather than emit escape codes.
    let home = TempDir::new().expect("tempdir");
    let out = bin(&home).arg("tui").assert().code(78);
    let stderr = String::from_utf8(out.get_output().stderr.clone()).expect("utf8");
    assert!(
        stderr.contains("requires an interactive terminal"),
        "expected non-tty rejection message, got: {stderr}"
    );
}
