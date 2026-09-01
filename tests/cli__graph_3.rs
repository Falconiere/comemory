#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The two node columns the console's selected-node panel needs, added in
//! the console-compat change (spec AC-29): `memories` (how many live
//! memories cite this file) and `blob` (the OID pinned at index time).
//!
//! Real data throughout: a real git repo, indexed by the real
//! `comemory index-code`, with memories written by the real `comemory save`
//! carrying backtick-fenced references that the cross-link writer turns into
//! `references_file` / `references_symbol` edges. The counts are checked
//! against the same facts read straight out of SQLite, so the test cannot
//! pass by agreeing with itself.

use std::path::Path;

use assert_cmd::Command;
use tempfile::TempDir;

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;

fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").unwrap();
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
}

/// Index a two-file repo and return its path.
fn index_repo(home: &TempDir, workspace: &Path, label: &str) -> std::path::PathBuf {
    let repo = workspace.join(label);
    git_repo::init_repo(&repo);
    git_commit::commit_files(
        &repo,
        &[
            ("a.rs", "mod b;\n\nfn alpha() {}\n"),
            ("b.rs", "fn beta() {}\n"),
        ],
        "initial",
    );
    bin(home)
        .args(["index-code", "--repo", label, "--path"])
        .arg(repo.as_os_str())
        .assert()
        .success();
    repo
}

fn graph_json(home: &TempDir) -> serde_json::Value {
    let out = bin(home).args(["graph", "--json"]).output().unwrap();
    assert!(
        out.status.success(),
        "graph failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("graph --json parses")
}

fn node<'a>(graph: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    graph["nodes"]
        .as_array()
        .expect("nodes array")
        .iter()
        .find(|n| n["id"] == id)
        .unwrap_or_else(|| panic!("node {id} not in graph: {graph}"))
}

#[test]
fn nodes_carry_the_blob_oid_recorded_at_index_time() {
    let home = TempDir::new().unwrap();
    let workspace = TempDir::new().unwrap();
    index_repo(&home, workspace.path(), "demo");

    let graph = graph_json(&home);
    let a = node(&graph, "file:demo:a.rs");

    let blob = a["blob"].as_str().expect("an indexed file has a blob oid");
    assert_eq!(blob.len(), 40, "a git blob oid is 40 hex chars: {blob}");

    // Cross-check against the row the indexer actually wrote.
    let db = home.path().join(".comemory").join("comemory.db");
    let conn = rusqlite::Connection::open(db).unwrap();
    let stored: String = conn
        .query_row(
            "SELECT blob_oid FROM indexed_files WHERE repo = 'demo' AND path = 'a.rs'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(blob, stored, "the node's blob is the indexed_files row");
}

#[test]
fn memories_counts_distinct_citing_memories_across_file_and_symbol_refs() {
    let home = TempDir::new().unwrap();
    let workspace = TempDir::new().unwrap();
    index_repo(&home, workspace.path(), "demo");

    // One memory cites the file; a second cites two symbols in the SAME
    // file, which must count once, not twice.
    bin(&home)
        .args([
            "save",
            "the alpha entry point lives in `demo:a.rs` and is the one caller",
            "--kind",
            "note",
        ])
        .assert()
        .success();
    bin(&home)
        .args([
            "save",
            "both `demo:a.rs:alpha` and `demo:a.rs:beta` were renamed in the same pass",
            "--kind",
            "decision",
        ])
        .assert()
        .success();
    // A memory citing a DIFFERENT file must not inflate a.rs's count.
    bin(&home)
        .args(["save", "`demo:b.rs` is only ever a leaf", "--kind", "note"])
        .assert()
        .success();

    let graph = graph_json(&home);

    assert_eq!(
        node(&graph, "file:demo:a.rs")["memories"],
        2,
        "two distinct memories cite a.rs — the one citing two of its symbols counts once"
    );
    assert_eq!(
        node(&graph, "file:demo:b.rs")["memories"],
        1,
        "b.rs is cited by exactly one memory"
    );
}

#[test]
fn a_soft_deleted_memory_stops_counting() {
    let home = TempDir::new().unwrap();
    let workspace = TempDir::new().unwrap();
    index_repo(&home, workspace.path(), "demo");

    let out = bin(&home)
        .args([
            "save",
            "`demo:a.rs` holds the entry point",
            "--kind",
            "note",
            "--json",
        ])
        .output()
        .unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let id = saved["id"].as_str().unwrap().to_string();

    assert_eq!(node(&graph_json(&home), "file:demo:a.rs")["memories"], 1);

    bin(&home).args(["delete", &id]).assert().success();

    assert_eq!(
        node(&graph_json(&home), "file:demo:a.rs")["memories"],
        0,
        "a soft-deleted memory must drop out of the node's citation count"
    );
}

#[test]
fn the_pre_existing_node_fields_are_unchanged() {
    let home = TempDir::new().unwrap();
    let workspace = TempDir::new().unwrap();
    index_repo(&home, workspace.path(), "demo");

    let graph = graph_json(&home);
    let a = node(&graph, "file:demo:a.rs");

    // Spec Non-Goal 4: growth is additive. Every field the graph emitted
    // before this change must still be there, with its old meaning.
    assert_eq!(a["id"], "file:demo:a.rs");
    assert_eq!(a["label"], "a.rs");
    assert_eq!(a["repo"], "demo");
    assert!(a["rank"].is_number(), "rank is still a number");
    assert!(
        a["symbols"].as_u64().unwrap() >= 1,
        "symbols still counts top-level symbols"
    );
}

#[test]
fn a_path_containing_an_underscore_does_not_match_a_sibling_file() {
    // Regression: the memory count matched symbol refs with
    // `dst_id LIKE '<repo>:<path>:%'`. In LIKE, `_` means "any single
    // character" — and Rust paths are full of underscores — so
    // `demo:memory_list.rs:%` also matched `demo:memoryXlist.rs:...`,
    // inflating one file's count with another file's citations.
    let home = TempDir::new().unwrap();
    let workspace = TempDir::new().unwrap();

    let repo = workspace.path().join("demo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(
        &repo,
        &[
            ("memory_list.rs", "fn list_memories() {}\n"),
            ("memoryXlist.rs", "fn decoy() {}\n"),
        ],
        "two files one underscore apart",
    );
    bin(&home)
        .args(["index-code", "--repo", "demo", "--path"])
        .arg(repo.as_os_str())
        .assert()
        .success();

    // Cite ONLY the decoy's symbol.
    bin(&home)
        .args([
            "save",
            "the decoy entry point is `demo:memoryXlist.rs:decoy`",
            "--kind",
            "note",
        ])
        .assert()
        .success();

    let graph = graph_json(&home);
    assert_eq!(
        node(&graph, "file:demo:memoryXlist.rs")["memories"],
        1,
        "the cited file counts its one memory"
    );
    assert_eq!(
        node(&graph, "file:demo:memory_list.rs")["memories"],
        0,
        "the underscore file was never cited — a LIKE wildcard must not \
         borrow its sibling's citation"
    );
}
