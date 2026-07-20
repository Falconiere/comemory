//! Pagination tests for `comemory context` — split from `cli__context.rs`.
//!
//! `context` returns a bundle; pagination applies to its primary memory
//! list (the `memories` array). Per-memory code refs are intentionally
//! left unpaginated — each surfaced memory keeps its full ref set. These
//! tests page the memory list and assert stability + the envelope cursor.

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
}

/// Distinct filler words per index so the seeded bodies do not collapse as
/// SimHash near-duplicates (which would shrink the diversified window).
const FILLER: &[&str] = &[
    "postgres",
    "sqlite",
    "redis",
    "kafka",
    "nginx",
    "docker",
    "kubernetes",
    "grpc",
    "graphql",
    "webpack",
    "tokio",
    "serde",
    "clap",
    "rayon",
    "hyper",
    "axum",
    "diesel",
    "sea",
    "rocket",
    "warp",
    "tonic",
    "tower",
];

/// Seed `n` memories that all match the `context` query lexically but carry
/// distinct filler vocabulary so each stays a distinct (non-collapsed) row.
fn seed_many(home: &TempDir, n: usize) {
    for i in 0..n {
        let w1 = FILLER[i % FILLER.len()];
        let w2 = FILLER[(i * 7 + 3) % FILLER.len()];
        bin(home)
            .args([
                "save",
                "--kind",
                "note",
                &format!("context paging note {i} about advisory locking {w1} {w2} pool ordering"),
            ])
            .assert()
            .success();
    }
}

/// Drive `context --json` with access tracking off so multi-call pagination
/// harnesses do not mutate ACT-R counters between pages (same hook as
/// `tests/cli__search.rs::search_json`).
fn context_json(home: &TempDir, extra: &[&str]) -> Value {
    let mut args = vec!["context", "advisory locking", "--json"];
    args.extend_from_slice(extra);
    let out = bin(home)
        .env("COMEMORY_DISABLE_ACCESS_TRACKING", "true")
        .args(&args)
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("json ({e}): {stdout}"))
}

fn memory_ids(v: &Value) -> Vec<String> {
    v["memories"]
        .as_array()
        .expect("memories array")
        .iter()
        .map(|m| m["id"].as_str().expect("id").to_string())
        .collect()
}

/// Paging the bundle's memory list is stable: no id repeats across pages,
/// none is skipped, and concatenated pages reproduce the single-shot
/// ranked memory window. The envelope carries the cursor fields.
#[test]
fn context_memory_list_pagination_is_stable() {
    let home = TempDir::new().expect("tempdir");
    seed_many(&home, 20);

    // Single-shot ground truth.
    let full = context_json(&home, &["--k", "0"]);
    let full_ids = memory_ids(&full);
    let total = full["total"].as_u64().expect("total") as usize;
    assert_eq!(full_ids.len(), total, "total == in-window ranked count");
    assert!(total > 10, "need > 2 pages of distinct memories: {total}");
    assert_eq!(
        full["has_more"],
        Value::Bool(false),
        "whole window: no more"
    );

    let page_size = 5;
    let mut seen = std::collections::HashSet::new();
    let mut joined: Vec<String> = Vec::new();
    let mut offset = 0;
    loop {
        let v = context_json(&home, &["--k", "5", "--offset", &offset.to_string()]);
        assert_eq!(v["limit"].as_u64(), Some(5), "limit echoes --k");
        assert_eq!(v["offset"].as_u64(), Some(offset as u64), "offset echoed");
        assert_eq!(v["total"].as_u64(), Some(total as u64), "stable total");
        for id in memory_ids(&v) {
            assert!(
                seen.insert(id.clone()),
                "memory {id} on two pages (overlap)"
            );
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
        "concatenated memory pages must reproduce the single-shot window"
    );
}

#[test]
fn context_limit_is_a_visible_alias_of_k() {
    let home = TempDir::new().expect("tempdir");
    seed_many(&home, 10);
    let via_k = memory_ids(&context_json(&home, &["--k", "3"]));
    let via_limit = memory_ids(&context_json(&home, &["--limit", "3"]));
    assert_eq!(via_k.len(), 3, "k bounds the memory page");
    assert_eq!(via_k, via_limit, "--limit must alias --k exactly");
}

#[test]
fn context_offset_beyond_window_is_empty_with_no_more() {
    let home = TempDir::new().expect("tempdir");
    seed_many(&home, 5);
    let v = context_json(&home, &["--k", "5", "--offset", "9999"]);
    assert!(
        memory_ids(&v).is_empty(),
        "offset past the window yields no memories"
    );
    assert_eq!(v["has_more"], Value::Bool(false), "nothing beyond");
}
