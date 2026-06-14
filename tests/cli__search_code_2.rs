//! Pagination tests for `comemory search-code` — split from
//! `cli__search_code.rs` to keep each test binary small. Indexes a real
//! Rust fixture with many functions, then pages through the ranked window
//! and asserts stability (no overlap, no gap, single-shot order) plus the
//! envelope cursor (`limit`/`offset`/`has_more`/`total`) and the
//! `--limit` alias.

use assert_cmd::Command;
use serde_json::Value;
use tempfile::tempdir;

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;

/// A Rust fixture with `n` distinct functions that all share the `handle`
/// subtoken so one query reaches every one of them.
fn build_many_fn_repo(root: &std::path::Path, n: usize) -> std::path::PathBuf {
    let repo = root.join("code-repo");
    git_repo::init_repo(&repo);
    let mut src = String::new();
    for i in 0..n {
        src.push_str(&format!("fn handle_request_{i}() {{}}\n"));
    }
    git_commit::commit_files(&repo, &[("lib.rs", &src)], "init");
    repo
}

fn index_repo(home: &tempfile::TempDir, repo: &std::path::Path) {
    Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(["index-code", "--repo", "r", "--path"])
        .arg(repo.as_os_str())
        .assert()
        .success();
}

fn search_code_json(home: &tempfile::TempDir, extra: &[&str]) -> Value {
    let mut args = vec!["search-code", "handle request", "--json"];
    args.extend_from_slice(extra);
    let assert = Command::cargo_bin("comemory")
        .expect("bin")
        .env("COMEMORY_DATA_DIR", home.path())
        .args(&args)
        .assert()
        .success();
    let out = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("json ({e}): {out}"))
}

fn symbol_ids(v: &Value) -> Vec<i64> {
    v["hits"]
        .as_array()
        .expect("hits array")
        .iter()
        .map(|h| h["symbol_id"].as_i64().expect("symbol_id"))
        .collect()
}

/// CLI-level stability for `search-code`: page through the ranked window
/// with `--k 6` and rising `--offset`. No symbol repeats across pages,
/// none is skipped, and the concatenation equals the single-shot window.
#[test]
fn search_code_pagination_is_stable_across_offsets() {
    let home = tempdir().expect("tempdir");
    let workspace = tempdir().expect("workspace");
    let repo = build_many_fn_repo(workspace.path(), 22);
    index_repo(&home, &repo);

    // Single-shot ground truth: the whole window.
    let full = search_code_json(&home, &["--k", "0"]);
    let full_ids = symbol_ids(&full);
    let total = full["total"].as_u64().expect("total") as usize;
    assert_eq!(full_ids.len(), total, "total == in-window ranked count");
    assert!(total > 12, "need > 2 pages of distinct hits: {total}");
    assert_eq!(
        full["has_more"],
        Value::Bool(false),
        "whole window: no more"
    );

    let page_size = 6;
    let mut seen = std::collections::HashSet::new();
    let mut joined: Vec<i64> = Vec::new();
    let mut offset = 0;
    loop {
        let v = search_code_json(&home, &["--k", "6", "--offset", &offset.to_string()]);
        assert_eq!(v["limit"].as_u64(), Some(6), "limit echoes --k");
        assert_eq!(v["offset"].as_u64(), Some(offset as u64), "offset echoed");
        assert_eq!(v["total"].as_u64(), Some(total as u64), "stable total");
        for id in symbol_ids(&v) {
            assert!(seen.insert(id), "symbol {id} on two pages (overlap)");
            joined.push(id);
        }
        let expect_more = offset + page_size < full_ids.len();
        assert_eq!(
            v["has_more"],
            Value::Bool(expect_more),
            "has_more wrong at offset {offset}"
        );
        if !expect_more {
            break;
        }
        offset += page_size;
    }
    assert_eq!(
        joined, full_ids,
        "concatenated pages must reproduce the single-shot ranked window"
    );
}

#[test]
fn search_code_limit_is_a_visible_alias_of_k() {
    let home = tempdir().expect("tempdir");
    let workspace = tempdir().expect("workspace");
    let repo = build_many_fn_repo(workspace.path(), 10);
    index_repo(&home, &repo);
    let via_k = symbol_ids(&search_code_json(&home, &["--k", "3"]));
    let via_limit = symbol_ids(&search_code_json(&home, &["--limit", "3"]));
    assert_eq!(via_k.len(), 3, "k bounds the page");
    assert_eq!(via_k, via_limit, "--limit must alias --k exactly");
}

#[test]
fn search_code_offset_beyond_window_is_empty_with_no_more() {
    let home = tempdir().expect("tempdir");
    let workspace = tempdir().expect("workspace");
    let repo = build_many_fn_repo(workspace.path(), 5);
    index_repo(&home, &repo);
    let v = search_code_json(&home, &["--k", "5", "--offset", "9999"]);
    assert!(symbol_ids(&v).is_empty(), "offset past the window is empty");
    assert_eq!(v["has_more"], Value::Bool(false), "nothing beyond");
}
