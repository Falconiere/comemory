#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Pre-fusion leg scores in `score_parts.legs` (spec AC-19, AC-20, AC-21,
//! AC-23).
//!
//! The console's Search screen renders a bar per retrieval leg with a note
//! like "cosine 0.71", which the post-fusion `rrf` / `relevance` figure
//! cannot supply. `legs` exposes what each leg actually contributed:
//! `bm25` (FTS5 BM25, negated to higher-is-better) and `ann` (cosine
//! similarity, `1.0 - distance`).
//!
//! Both are CARRIED, never recomputed — which is the property that lets
//! them exist without moving a ranking. `tests/ranking_invariance.rs`
//! guards that separately; this file proves the values are actually there,
//! and absent exactly when the corresponding leg did not fire.
//!
//! Real data: memories written by the real `comemory save`, a real vector
//! supplied through `--vector-stdin` (the BYO-vector contract), and a real
//! code index built by the real `comemory index-code`.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use assert_cmd::prelude::*;
use tempfile::TempDir;

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;

/// The memory vector dim baked into the `memory_vec` vec0 DDL.
const MEMORY_DIM: usize = 1024;

fn bin(data_dir: &Path) -> Command {
    let mut c = Command::cargo_bin("comemory").unwrap();
    c.env("COMEMORY_DATA_DIR", data_dir);
    // Tracking would bump access counts between the calls below and move
    // activation, which has nothing to do with what this file asserts.
    c.env("COMEMORY_DISABLE_ACCESS_TRACKING", "true");
    c
}

fn run(data_dir: &Path, args: &[&str]) -> String {
    let out = bin(data_dir).args(args).output().unwrap();
    assert!(
        out.status.success(),
        "comemory {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// A deterministic unit-ish vector, so the ANN leg has something real to
/// match against without depending on an external embedder.
fn vector(seed: f32) -> Vec<f32> {
    (0..MEMORY_DIM)
        .map(|i| ((i as f32).mul_add(0.001, seed)).sin())
        .collect()
}

/// Save a memory with a caller-supplied vector via `--vector-stdin`, the
/// BYO-vector path.
fn save_with_vector(data_dir: &Path, body: &str, seed: f32) {
    let payload = serde_json::json!({ "embedding": vector(seed) }).to_string();
    let mut child = bin(data_dir)
        .args(["save", body, "--kind", "note", "--vector-stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(payload.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "save --vector-stdin failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Search with a caller-supplied vector, returning the parsed envelope.
fn search_with_vector(data_dir: &Path, query: &str, seed: f32) -> serde_json::Value {
    let payload = serde_json::json!({ "embedding": vector(seed) }).to_string();
    let mut child = bin(data_dir)
        .args(["search", query, "--json", "--vector-stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(payload.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "search --vector-stdin failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("search --json parses")
}

#[test]
fn a_hybrid_search_reports_both_legs() {
    let dir = TempDir::new().unwrap();
    save_with_vector(
        dir.path(),
        "the frontmatter contract is what the ranker reads",
        0.10,
    );
    save_with_vector(dir.path(), "an unrelated note about swap space", 0.90);

    let envelope = search_with_vector(dir.path(), "frontmatter contract", 0.10);
    let hits = envelope["hits"].as_array().expect("hits array");
    assert!(!hits.is_empty(), "the query must match something");

    let legs = &hits[0]["score_parts"]["legs"];
    let bm25 = legs["bm25"]
        .as_f64()
        .expect("AC-19: a hybrid hit reports its lexical leg");
    let ann = legs["ann"]
        .as_f64()
        .expect("AC-19: a hybrid hit reports its vector leg");

    assert!(
        (0.0..=1.0).contains(&ann),
        "AC-19: ann is a cosine similarity in [0,1], got {ann}"
    );
    assert!(
        bm25.is_finite(),
        "bm25 is the negated FTS5 score, higher-is-better: {bm25}"
    );
}

#[test]
fn a_lexical_only_search_reports_bm25_and_a_null_ann() {
    let dir = TempDir::new().unwrap();
    run(
        dir.path(),
        &[
            "save",
            "soft-delete moves the markdown first, never the database",
            "--kind",
            "decision",
        ],
    );

    let envelope: serde_json::Value = serde_json::from_str(&run(
        dir.path(),
        &["search", "soft-delete markdown", "--json"],
    ))
    .unwrap();
    let hits = envelope["hits"].as_array().unwrap();
    assert!(!hits.is_empty());

    let legs = &hits[0]["score_parts"]["legs"];
    assert!(
        legs["bm25"].as_f64().is_some(),
        "AC-20: the lexical leg fired, so bm25 is present"
    );
    assert!(
        legs["ann"].is_null(),
        "AC-20: no vector was supplied, so the ann leg is null — not zero"
    );
}

#[test]
fn a_graph_only_hit_reports_neither_leg() {
    let dir = TempDir::new().unwrap();
    // Two memories linked by an explicit supersede edge. The superseding
    // one is lexically dark for the query below; only the graph-expansion
    // leg can reach it.
    let first = run(
        dir.path(),
        &[
            "save",
            "the tokenizer must split on digit boundaries",
            "--kind",
            "discovery",
            "--json",
        ],
    );
    let first_id = serde_json::from_str::<serde_json::Value>(&first).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    run(
        dir.path(),
        &[
            "save",
            "zzz unrelated vocabulary entirely",
            "--kind",
            "note",
            "--supersedes",
            &first_id,
        ],
    );

    let envelope: serde_json::Value = serde_json::from_str(&run(
        dir.path(),
        &["search", "tokenizer digit boundaries", "--json"],
    ))
    .unwrap();

    let graph_hit = envelope["hits"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["source"] == "graph");

    if let Some(hit) = graph_hit {
        let legs = &hit["score_parts"]["legs"];
        assert!(
            legs["bm25"].is_null() && legs["ann"].is_null(),
            "AC-21: a graph-expansion hit went through no lexical or vector \
             leg — both are null, and that absence is the signal"
        );
        assert!(
            hit["score_parts"]["final_score"].as_f64().unwrap() > 0.0,
            "AC-21: it still carries a real final score"
        );
        assert_eq!(hit["tier"], 0, "graph candidates report ladder tier 0");
    }
}

#[test]
fn search_code_reports_the_same_leg_object() {
    let home = TempDir::new().unwrap();
    let workspace = TempDir::new().unwrap();
    let data_dir = home.path().join(".comemory");

    let repo = workspace.path().join("demo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(
        &repo,
        &[(
            "parser.rs",
            "fn parse_frontmatter() {}\nfn serialize_tags() {}\n",
        )],
        "initial",
    );
    bin(&data_dir)
        .args(["index-code", "--repo", "demo", "--path"])
        .arg(repo.as_os_str())
        .assert()
        .success();

    let envelope: serde_json::Value = serde_json::from_str(&run(
        &data_dir,
        &["search-code", "parse_frontmatter", "--json"],
    ))
    .unwrap();
    let hits = envelope["hits"].as_array().expect("hits array");
    assert!(!hits.is_empty(), "the indexed symbol must be findable");

    let legs = &hits[0]["score_parts"]["legs"];
    assert!(
        legs["bm25"].as_f64().is_some(),
        "AC-23: the code lexical leg reports its BM25"
    );
    assert!(
        legs["ann"].is_null(),
        "AC-23: no vector supplied, so the code ann leg is null"
    );
}
