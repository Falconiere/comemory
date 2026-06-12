//! Integration tests for `comemory graph`. An empty index must still
//! render a well-formed (empty) graph in every output format — the command
//! is a pure read over `comemory.db` and never indexes.

use assert_cmd::Command;
use comemory::cli::graph::{build_graph, parse_id};
use comemory::output::graph::Edge;
use tempfile::TempDir;

use super::git_setup;

/// Index a two-file repo under `label` into `home`'s data dir: `a.rs`
/// declares `mod b;` (an import a.rs → b.rs) and a.rs+b.rs are committed
/// together twice, so the co-change miner records the pair with weight 2.
fn index_repo(home: &TempDir, workspace: &std::path::Path, label: &str) {
    let repo = workspace.join(label);
    git_setup::init_repo(&repo);
    git_setup::commit_files(
        &repo,
        &[
            ("a.rs", "mod b;\n\nfn alpha() {}\n"),
            ("b.rs", "fn beta() {}\n"),
        ],
        "couple once",
    );
    git_setup::commit_files(
        &repo,
        &[
            ("a.rs", "mod b;\n\nfn alpha() { let _x = 1; }\n"),
            ("b.rs", "fn beta() { let _y = 2; }\n"),
        ],
        "couple twice",
    );
    bin(home)
        .args(["index-code", "--repo", label, "--path"])
        .arg(repo.as_os_str())
        .assert()
        .success();
}

/// Run `comemory graph <extra…> --json` against `home` and parse the result.
fn graph_json(home: &TempDir, extra: &[&str]) -> serde_json::Value {
    let mut args = vec!["graph"];
    args.extend_from_slice(extra);
    args.push("--json");
    let out = bin(home).args(&args).assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).expect("utf8");
    serde_json::from_str(stdout.trim()).expect("valid json")
}

#[test]
fn graph_emits_indexed_edges_and_gates_co_changed_weight() {
    let home = TempDir::new().expect("tempdir");
    let ws = TempDir::new().expect("workspace");
    index_repo(&home, ws.path(), "r");

    let v = graph_json(&home, &["--repo", "r"]);
    let ids: Vec<&str> = v["nodes"]
        .as_array()
        .expect("nodes array")
        .iter()
        .map(|n| n["id"].as_str().expect("id string"))
        .collect();
    assert!(ids.contains(&"file:r:a.rs"), "a.rs node present");
    assert!(ids.contains(&"file:r:b.rs"), "b.rs node present");

    let edges = v["edges"].as_array().expect("edges array");
    let co = edges
        .iter()
        .find(|e| e["rel"] == "co_changed")
        .expect("co_changed edge present");
    assert_eq!(co["weight"], 2, "two coupling commits → weight 2");
    assert!(
        edges.iter().any(|e| e["rel"] == "imports"),
        "imports edge present"
    );

    // `--min-weight 3` drops the weight-2 co_changed edge but leaves the
    // weight-1 imports edge untouched (the floor only gates co_changed).
    let gated = graph_json(&home, &["--repo", "r", "--min-weight", "3"]);
    let gated_edges = gated["edges"].as_array().expect("edges array");
    assert!(
        !gated_edges.iter().any(|e| e["rel"] == "co_changed"),
        "weight-2 co_changed gated out by --min-weight 3"
    );
    assert!(
        gated_edges.iter().any(|e| e["rel"] == "imports"),
        "imports survive --min-weight"
    );
}

#[test]
fn graph_repo_filter_excludes_other_repos() {
    let home = TempDir::new().expect("tempdir");
    let ws = TempDir::new().expect("workspace");
    index_repo(&home, ws.path(), "r");
    index_repo(&home, ws.path(), "r2");

    // Scoped export: every node and both edge endpoints stay within `r`.
    let scoped = graph_json(&home, &["--repo", "r"]);
    for n in scoped["nodes"].as_array().expect("nodes array") {
        let id = n["id"].as_str().expect("id string");
        assert!(
            id.starts_with("file:r:"),
            "node {id} leaked from another repo"
        );
    }
    for e in scoped["edges"].as_array().expect("edges array") {
        assert!(e["src"].as_str().expect("src").starts_with("file:r:"));
        assert!(e["dst"].as_str().expect("dst").starts_with("file:r:"));
    }

    // Unscoped export sees both repos.
    let all = graph_json(&home, &[]);
    let repos: std::collections::HashSet<&str> = all["nodes"]
        .as_array()
        .expect("nodes array")
        .iter()
        .map(|n| n["repo"].as_str().expect("repo string"))
        .collect();
    assert!(
        repos.contains("r") && repos.contains("r2"),
        "both repos in unscoped graph"
    );
}

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
fn html_format_loads_sigma() {
    let home = TempDir::new().expect("tempdir");
    let out = bin(&home)
        .args(["graph", "--format", "html"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).expect("utf8");
    assert!(stdout.contains("sigma"), "viewer must embed sigma.js");
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
