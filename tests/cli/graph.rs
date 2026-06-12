//! Integration tests for `comemory graph`. An empty index must still
//! render a well-formed (empty) graph in every output format — the command
//! is a pure read over `comemory.db` and never indexes.

use assert_cmd::Command;
use tempfile::TempDir;

/// Build a `comemory` invocation with `COMEMORY_DATA_DIR` rooted at `home`.
fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
}

#[test]
fn empty_index_renders_empty_json_graph() {
    let home = TempDir::new().expect("tempdir");
    let out = bin(&home).args(["graph"]).assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).expect("utf8");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid json");
    assert_eq!(v["nodes"].as_array().expect("nodes array").len(), 0);
    assert_eq!(v["edges"].as_array().expect("edges array").len(), 0);
}

#[test]
fn dot_format_emits_digraph_header() {
    let home = TempDir::new().expect("tempdir");
    let out = bin(&home)
        .args(["graph", "--format", "dot"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).expect("utf8");
    assert!(stdout.starts_with("digraph comemory {"), "got: {stdout}");
}

#[test]
fn html_format_loads_cytoscape() {
    let home = TempDir::new().expect("tempdir");
    let out = bin(&home)
        .args(["graph", "--format", "html"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).expect("utf8");
    assert!(stdout.contains("cytoscape"), "viewer must embed cytoscape");
    assert!(!stdout.contains("__GRAPH_DATA__"), "data must be inlined");
}

#[test]
fn global_json_flag_forces_json_over_format() {
    // `--json` (global) wins over `--format dot`: the dispatcher forces the
    // JSON renderer so machine callers get a parseable envelope regardless.
    let home = TempDir::new().expect("tempdir");
    let out = bin(&home)
        .args(["graph", "--format", "dot", "--json"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).expect("utf8");
    serde_json::from_str::<serde_json::Value>(stdout.trim()).expect("json output");
}

#[test]
fn rejects_zero_min_weight() {
    let home = TempDir::new().expect("tempdir");
    bin(&home)
        .args(["graph", "--min-weight", "0"])
        .assert()
        .failure();
}
