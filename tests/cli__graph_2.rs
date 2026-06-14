//! Pagination coverage for `comemory graph` (continuation of
//! `cli__graph.rs`, split to stay within the per-file size budget). Exercises
//! the `--limit` / `--offset` edge window: the stable `weight DESC` order, an
//! exact `total` / `has_more`, nodes derived from only the paged edges (no
//! dangling, no extras), `--limit 0` = full graph, scope-filter composition,
//! and the windowed DOT export with its stderr footer.

use assert_cmd::Command;
use tempfile::TempDir;

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;

/// Build a `comemory` invocation with `COMEMORY_DATA_DIR` rooted at `home`.
fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
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

/// Index a three-file repo under `label`: `a.rs` declares `mod b;` (one
/// import edge a.rs→b.rs) and a.rs+b.rs are committed together twice (co-change
/// pair, weight 2). `c.rs` is committed alone and imports nothing, so it is an
/// indexed node with NO edges — used to prove a paged subgraph carries only the
/// endpoints of its windowed edges, never every indexed file.
fn index_three_file_repo(home: &TempDir, workspace: &std::path::Path, label: &str) {
    let repo = workspace.join(label);
    git_repo::init_repo(&repo);
    git_commit::commit_files(
        &repo,
        &[
            ("a.rs", "mod b;\n\nfn alpha() {}\n"),
            ("b.rs", "fn beta() {}\n"),
        ],
        "couple once",
    );
    git_commit::commit_files(
        &repo,
        &[
            ("a.rs", "mod b;\n\nfn alpha() { let _x = 1; }\n"),
            ("b.rs", "fn beta() { let _y = 2; }\n"),
        ],
        "couple twice",
    );
    // c.rs lands in its own commit and imports nothing → no edges touch it.
    git_commit::commit_files(&repo, &[("c.rs", "fn gamma() {}\n")], "lonely c");
    bin(home)
        .args(["index-code", "--repo", label, "--path"])
        .arg(repo.as_os_str())
        .assert()
        .success();
}

/// All `(src, dst, rel, weight)` tuples in a graph-page/graph JSON value, in
/// emission order, so a test can assert the stable `weight DESC` ordering.
fn edge_tuples(v: &serde_json::Value) -> Vec<(String, String, String, i64)> {
    v["edges"]
        .as_array()
        .expect("edges array")
        .iter()
        .map(|e| {
            (
                e["src"].as_str().expect("src").to_string(),
                e["dst"].as_str().expect("dst").to_string(),
                e["rel"].as_str().expect("rel").to_string(),
                e["weight"].as_i64().expect("weight"),
            )
        })
        .collect()
}

/// The set of node ids in a graph-page/graph JSON value.
fn node_ids(v: &serde_json::Value) -> std::collections::BTreeSet<String> {
    v["nodes"]
        .as_array()
        .expect("nodes array")
        .iter()
        .map(|n| n["id"].as_str().expect("id").to_string())
        .collect()
}

#[test]
fn graph_paginates_edges_by_stable_weight_order() {
    let home = TempDir::new().expect("tempdir");
    let ws = TempDir::new().expect("workspace");
    index_three_file_repo(&home, ws.path(), "r");

    // Full window first: exactly two edges (co_changed weight 2, imports
    // weight 1), and the lonely c.rs node is NOT an edge endpoint.
    let full = graph_json(&home, &["--repo", "r", "--limit", "0"]);
    assert_eq!(full["total"], 2, "two edges total");
    assert_eq!(full["has_more"], false, "--limit 0 returns the full graph");
    assert_eq!(edge_tuples(&full).len(), 2);

    // First page of one edge: the strongest (co_changed, weight 2) leads under
    // the `weight DESC` order; total/has_more are exact; nodes are exactly its
    // two endpoints — c.rs (an indexed node with no edges) is absent.
    let p0 = graph_json(&home, &["--repo", "r", "--limit", "1", "--offset", "0"]);
    let e0 = edge_tuples(&p0);
    assert_eq!(e0.len(), 1, "limit 1 → one edge");
    assert_eq!(e0[0].2, "co_changed", "strongest edge leads (weight DESC)");
    assert_eq!(e0[0].3, 2);
    assert_eq!(p0["total"], 2, "total counts all matching edges");
    assert_eq!(p0["offset"], 0);
    assert_eq!(p0["limit"], 1);
    assert_eq!(p0["has_more"], true, "one of two shown → more remain");
    assert_eq!(
        node_ids(&p0),
        ["file:r:a.rs".to_string(), "file:r:b.rs".to_string()]
            .into_iter()
            .collect(),
        "nodes are exactly the paged edge's endpoints — no c.rs, no extras"
    );

    // Second page: the weaker imports edge, last window → has_more false.
    let p1 = graph_json(&home, &["--repo", "r", "--limit", "1", "--offset", "1"]);
    let e1 = edge_tuples(&p1);
    assert_eq!(e1.len(), 1);
    assert_eq!(e1[0].2, "imports", "weaker edge on the second page");
    assert_eq!(e1[0].3, 1);
    assert_eq!(p1["has_more"], false, "offset 1 + 1 shown == 2 total");

    // Offset past the end → empty edges, but total/has_more stay coherent.
    let past = graph_json(&home, &["--repo", "r", "--limit", "1", "--offset", "5"]);
    assert_eq!(edge_tuples(&past).len(), 0, "offset past end → empty page");
    assert_eq!(past["total"], 2);
    assert_eq!(past["has_more"], false);
    assert!(node_ids(&past).is_empty(), "no edges → no derived nodes");
}

#[test]
fn graph_pagination_composes_with_scope_filters() {
    let home = TempDir::new().expect("tempdir");
    let ws = TempDir::new().expect("workspace");
    index_three_file_repo(&home, ws.path(), "r");
    index_three_file_repo(&home, ws.path(), "r2");

    // `--repo` scopes the count: total is r's 2 edges, not r+r2's 4.
    let scoped = graph_json(&home, &["--repo", "r", "--limit", "1"]);
    assert_eq!(scoped["total"], 2, "--repo bounds the edge total");
    for id in node_ids(&scoped) {
        assert!(id.starts_with("file:r:"), "node {id} leaked from r2");
    }

    // `--min-weight 3` drops the weight-2 co_changed edge before paging (the
    // floor gates only co_changed; the weight-1 imports edge is untouched), so
    // the total collapses to the single surviving imports edge.
    let gated = graph_json(
        &home,
        &["--repo", "r", "--min-weight", "3", "--limit", "10"],
    );
    assert_eq!(gated["total"], 1, "min-weight filters before the window");
    let ge = edge_tuples(&gated);
    assert_eq!(ge.len(), 1);
    assert_eq!(ge[0].2, "imports", "only the ungated imports edge survives");
    assert_eq!(gated["has_more"], false);
}

#[test]
fn graph_dot_window_bounds_edges_and_footer_to_stderr() {
    let home = TempDir::new().expect("tempdir");
    let ws = TempDir::new().expect("workspace");
    index_three_file_repo(&home, ws.path(), "r");

    // DOT honors the same window: a `--limit 1` export renders one edge, and
    // the pagination footer goes to stderr (never into the DOT on stdout).
    let out = bin(&home)
        .args(["graph", "--repo", "r", "--format", "dot", "--limit", "1"])
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).expect("utf8");
    let stderr = String::from_utf8(out.get_output().stderr.clone()).expect("utf8");
    assert!(stdout.starts_with("digraph comemory {"), "dot header");
    let edge_lines = stdout.matches(" -> ").count();
    assert_eq!(edge_lines, 1, "windowed DOT renders exactly one edge");
    assert!(
        stderr.contains("showing 1") && stderr.contains("of 2"),
        "window footer on stderr: {stderr:?}"
    );
    assert!(
        !stdout.contains("showing"),
        "footer must not pollute the DOT pipeline"
    );
}
