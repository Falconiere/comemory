//! Integration tests for `comemory graph`. An empty index must still
//! render a well-formed (empty) graph in every output format — the command
//! is a pure read over `comemory.db` and never indexes.

use assert_cmd::Command;
use comemory::cli::graph::{build_graph, parse_id};
use comemory::output::graph::Edge;
use tempfile::TempDir;

#[test]
fn build_graph_materializes_dangling_edge_endpoints() {
    // A `co_changed` edge to a file with no `code_symbols` row (e.g. a
    // deleted file) must still yield a zero-rank node so the edge is not
    // orphaned. Only the source node is backed by a real row here.
    let nodes = vec![("demo".into(), "src/a.rs".into(), 0.7, 2)];
    let edges = vec![Edge {
        src: "file:demo:src/a.rs".into(),
        dst: "file:demo:src/gone.rs".into(),
        rel: "co_changed".into(),
        weight: 3,
    }];
    let g = build_graph(nodes, edges);

    assert_eq!(g.nodes.len(), 2, "dangling dst must be materialized");
    let dangling = g
        .nodes
        .iter()
        .find(|n| n.id == "file:demo:src/gone.rs")
        .expect("dangling node present");
    assert_eq!(dangling.rank, 0.0, "dangling node is zero-rank");
    assert_eq!(dangling.symbols, 0, "dangling node has no symbols");
    assert_eq!(dangling.label, "src/gone.rs");
    assert_eq!(dangling.repo, "demo");
}

#[test]
fn parse_id_splits_repo_and_path() {
    assert_eq!(
        parse_id("file:demo:src/a.rs"),
        Some(("demo", "src/a.rs")),
        "well-formed id splits into (repo, path)"
    );
    assert_eq!(
        parse_id("file:demo:src/dir:weird.rs"),
        Some(("demo", "src/dir:weird.rs"))
    );
    assert_eq!(parse_id("notfile:demo:x"), None, "wrong prefix rejected");
    assert_eq!(
        parse_id("file:demo"),
        None,
        "missing path separator rejected"
    );
}

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
