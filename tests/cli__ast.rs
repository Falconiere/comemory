//! `comemory ast` integration tests: `--lang` gating (only the five
//! compiled-in languages are accepted) plus the pagination contract — the
//! `--json` output is now the shared `Page` envelope (not a bare array), and
//! `--limit`/`--offset` window the matches with a correct `has_more`.

use std::io::Write as _;

use assert_cmd::Command;

#[test]
fn ast_rejects_unsupported_lang() {
    // `--file` is required by clap so we point at a non-existent path; the
    // `--lang` guard must fire before any file IO so the test stays hermetic.
    let bogus_file = std::env::temp_dir().join("comemory-ast-lang-guard.rs");
    let assertion = Command::cargo_bin("comemory")
        .expect("cargo_bin comemory")
        .args(["ast", "pattern", "--lang", "ruby", "--file"])
        .arg(&bogus_file)
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assertion.get_output().stderr).to_string();
    assert!(
        stderr.contains("supported:"),
        "stderr should list supported langs, got: {stderr:?}"
    );
}

/// Write a Rust source file with `n` distinct `tokio::spawn(...)` call sites
/// (one per line) into a unique temp path, returning the path. Real source so
/// the ast-grep grammar produces real matches — no mocks.
fn spawn_fixture(tag: &str, n: usize) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("comemory-ast-page-{tag}-{n}.rs"));
    let mut f = std::fs::File::create(&path).expect("create fixture");
    for i in 0..n {
        writeln!(f, "fn t{i}() {{ tokio::spawn(async {{ {i} }}); }}").expect("write fixture line");
    }
    path
}

fn run_ast_json(file: &std::path::Path, extra: &[&str]) -> serde_json::Value {
    let mut cmd = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    cmd.args(["ast", "tokio::spawn($$$)", "--lang", "rs", "--file"])
        .arg(file)
        .arg("--json")
        .args(extra);
    let out = cmd.assert().success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("ast --json not valid JSON: {e}\n{stdout}"))
}

#[test]
fn ast_json_is_page_envelope_not_bare_array() {
    let file = spawn_fixture("envelope", 3);
    let v = run_ast_json(&file, &[]);
    // The new contract is an object with the Page fields, not a bare array.
    assert!(v.is_object(), "ast --json must be a Page object, got: {v}");
    assert!(v.get("items").map(|i| i.is_array()).unwrap_or(false));
    assert_eq!(v["items"].as_array().expect("items array").len(), 3);
    assert_eq!(v["total"], serde_json::json!(3));
    // Default limit (50) > 3 matches, so no further pages.
    assert_eq!(v["has_more"], serde_json::json!(false));
    // Each item carries the (line, text) row shape.
    let first = &v["items"][0];
    assert!(first.get("line").is_some(), "row must carry line: {first}");
    assert!(first.get("text").is_some(), "row must carry text: {first}");
}

#[test]
fn ast_json_limit_and_offset_slice_matches() {
    let file = spawn_fixture("slice", 5);
    let v = run_ast_json(&file, &["--limit", "2", "--offset", "1"]);
    assert_eq!(v["items"].as_array().expect("items").len(), 2);
    assert_eq!(v["limit"], serde_json::json!(2));
    assert_eq!(v["offset"], serde_json::json!(1));
    assert_eq!(v["total"], serde_json::json!(5));
    // offset 1 + 2 shown = 3 < 5 total -> more pages remain.
    assert_eq!(v["has_more"], serde_json::json!(true));
}

#[test]
fn ast_json_last_page_has_no_more() {
    let file = spawn_fixture("lastpage", 5);
    let v = run_ast_json(&file, &["--limit", "2", "--offset", "4"]);
    assert_eq!(v["items"].as_array().expect("items").len(), 1);
    assert_eq!(v["has_more"], serde_json::json!(false));
    assert_eq!(v["total"], serde_json::json!(5));
}

#[test]
fn ast_json_limit_zero_returns_all() {
    let file = spawn_fixture("all", 4);
    let v = run_ast_json(&file, &["--limit", "0"]);
    assert_eq!(v["items"].as_array().expect("items").len(), 4);
    assert_eq!(v["limit"], serde_json::json!(0));
    assert_eq!(v["has_more"], serde_json::json!(false));
}

#[test]
fn ast_tty_prints_pagination_footer() {
    let file = spawn_fixture("tty", 3);
    let out = Command::cargo_bin("comemory")
        .expect("cargo_bin comemory")
        .args(["ast", "tokio::spawn($$$)", "--lang", "rs", "--file"])
        .arg(&file)
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(
        stdout.contains("showing 1\u{2013}3 of 3 (--offset 0)"),
        "TTY footer missing; got: {stdout:?}"
    );
}
