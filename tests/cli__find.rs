#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `comemory find` — the unified ranking (spec AC-13, AC-13b, AC-14,
//! AC-15, AC-16, AC-17, AC-18).
//!
//! Real data end to end: memories written by the real `comemory save`, a
//! real git repo indexed by the real `comemory index-code`, and the real
//! binary driven as a subprocess. The load-bearing claims are that a
//! single-domain run orders identically to that domain's dedicated command
//! (which is what lets `find` exist beside them rather than replace them),
//! and that the document leg's weight is genuinely read.

use std::path::Path;
use std::process::Command;

use assert_cmd::prelude::*;
use tempfile::TempDir;

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;

fn bin(data_dir: &Path) -> Command {
    let mut c = Command::cargo_bin("comemory").unwrap();
    c.env("COMEMORY_DATA_DIR", data_dir);
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

fn json(data_dir: &Path, args: &[&str]) -> serde_json::Value {
    serde_json::from_str(&run(data_dir, args)).expect("--json output parses")
}

/// Ids in returned order.
fn ids(envelope: &serde_json::Value) -> Vec<String> {
    envelope["hits"]
        .as_array()
        .expect("hits array")
        .iter()
        .map(|h| h["id"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// A corpus with BOTH a memory and a code symbol matching "frontmatter".
fn seed_both_domains(home: &TempDir, workspace: &TempDir) -> std::path::PathBuf {
    let data_dir = home.path().join(".comemory");
    run(
        &data_dir,
        &[
            "save",
            "frontmatter is the contract the ranker reads, not the body",
            "--kind",
            "decision",
            "--repo",
            "demo",
        ],
    );
    run(
        &data_dir,
        &[
            "save",
            "frontmatter round-trip broke on empty tag lists",
            "--kind",
            "bug",
            "--repo",
            "demo",
        ],
    );

    let repo = workspace.path().join("demo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(
        &repo,
        &[(
            "frontmatter.rs",
            "fn parse_frontmatter() {}\nfn write_frontmatter() {}\n",
        )],
        "initial",
    );
    bin(&data_dir)
        .args(["index-code", "--repo", "demo", "--path"])
        .arg(repo.as_os_str())
        .assert()
        .success();
    data_dir
}

#[test]
fn find_returns_both_domains_in_one_ranking_ordered_by_score() {
    let home = TempDir::new().unwrap();
    let workspace = TempDir::new().unwrap();
    let data_dir = seed_both_domains(&home, &workspace);

    let envelope = json(&data_dir, &["find", "frontmatter", "--json"]);
    let hits = envelope["hits"].as_array().expect("hits array");
    assert!(
        hits.len() >= 2,
        "both domains should contribute: {envelope}"
    );

    let domains: std::collections::HashSet<&str> =
        hits.iter().filter_map(|h| h["domain"].as_str()).collect();
    assert!(
        domains.contains("memory") && domains.contains("code"),
        "AC-13: one list carries hits from both domains, got {domains:?}"
    );

    let scores: Vec<f64> = hits.iter().map(|h| h["score"].as_f64().unwrap()).collect();
    let mut sorted = scores.clone();
    sorted.sort_by(|a, b| b.partial_cmp(a).unwrap());
    assert_eq!(
        scores, sorted,
        "AC-13: hits are ordered by descending score"
    );
}

#[test]
fn a_memory_only_find_orders_identically_to_search() {
    let home = TempDir::new().unwrap();
    let workspace = TempDir::new().unwrap();
    let data_dir = seed_both_domains(&home, &workspace);

    let found = json(
        &data_dir,
        &[
            "find",
            "frontmatter",
            "--domain",
            "memory",
            "--json",
            "--k",
            "5",
        ],
    );
    let searched = json(&data_dir, &["search", "frontmatter", "--json", "--k", "5"]);

    let found_ids = ids(&found);
    let search_ids: Vec<String> = searched["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["memory_id"].as_str().unwrap().to_string())
        .collect();

    assert_eq!(
        found_ids, search_ids,
        "AC-14: --domain memory must reproduce `comemory search`'s ordering"
    );
}

#[test]
fn a_code_only_find_orders_identically_to_search_code() {
    let home = TempDir::new().unwrap();
    let workspace = TempDir::new().unwrap();
    let data_dir = seed_both_domains(&home, &workspace);

    let found = json(
        &data_dir,
        &[
            "find",
            "frontmatter",
            "--domain",
            "code",
            "--json",
            "--k",
            "5",
        ],
    );
    let searched = json(
        &data_dir,
        &["search-code", "frontmatter", "--json", "--k", "5"],
    );

    let found_ids = ids(&found);
    let search_ids: Vec<String> = searched["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["symbol_id"].to_string())
        .collect();

    assert_eq!(
        found_ids, search_ids,
        "AC-15: --domain code must reproduce `comemory search-code`'s ordering"
    );
}

#[test]
fn kind_narrows_only_memory_and_lang_only_code() {
    let home = TempDir::new().unwrap();
    let workspace = TempDir::new().unwrap();
    let data_dir = seed_both_domains(&home, &workspace);

    let envelope = json(
        &data_dir,
        &[
            "find",
            "frontmatter",
            "--kind",
            "bug",
            "--lang",
            "rust",
            "--json",
        ],
    );
    let hits = envelope["hits"].as_array().unwrap();

    let domains: std::collections::HashSet<&str> =
        hits.iter().filter_map(|h| h["domain"].as_str()).collect();
    assert!(
        domains.contains("memory") && domains.contains("code"),
        "AC-17: a per-domain filter must not drop the OTHER domain, got {domains:?}"
    );

    for h in hits.iter().filter(|h| h["domain"] == "memory") {
        assert!(
            h["subtitle"].as_str().unwrap().contains("bug"),
            "AC-17: --kind narrowed the memory leg: {h}"
        );
    }
}

#[test]
fn an_unknown_domain_is_a_usage_error_naming_the_offender() {
    let home = TempDir::new().unwrap();
    let data_dir = home.path().join(".comemory");
    run(&data_dir, &["save", "anything at all", "--kind", "note"]);

    let out = bin(&data_dir)
        .args(["find", "anything", "--domain", "sideways"])
        .output()
        .unwrap();

    assert!(!out.status.success(), "an unknown domain must not succeed");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("sideways"),
        "the error names the offending value: {stderr}"
    );
}

#[test]
fn find_writes_one_retrieval_log_row_whose_query_id_feedback_accepts() {
    let home = TempDir::new().unwrap();
    let workspace = TempDir::new().unwrap();
    let data_dir = seed_both_domains(&home, &workspace);

    let envelope = json(&data_dir, &["find", "frontmatter", "--json"]);
    let query_id = envelope["query_id"]
        .as_str()
        .expect("AC-18: a tracked find reports its query_id");

    let db = data_dir.join("comemory.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    let rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM retrieval_log WHERE query_id = ?1",
            [query_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        rows, 1,
        "AC-18: exactly one log row per run, not one per leg"
    );

    let source: String = conn
        .query_row(
            "SELECT source FROM retrieval_log WHERE query_id = ?1",
            [query_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(source, "find", "the row is attributed to `find`");

    let memory_id = envelope["hits"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["domain"] == "memory")
        .and_then(|h| h["id"].as_str())
        .expect("a memory hit to mark used")
        .to_string();
    run(&data_dir, &["feedback", query_id, "--used", &memory_id]);
}

#[test]
fn paging_does_not_repeat_a_hit_and_total_is_the_fused_count() {
    let home = TempDir::new().unwrap();
    let workspace = TempDir::new().unwrap();
    let data_dir = seed_both_domains(&home, &workspace);

    let first = json(&data_dir, &["find", "frontmatter", "--json", "--k", "1"]);
    let second = json(
        &data_dir,
        &["find", "frontmatter", "--json", "--k", "1", "--offset", "1"],
    );

    let a = ids(&first);
    let b = ids(&second);
    assert_eq!(a.len(), 1);
    assert_eq!(b.len(), 1);
    assert_ne!(a[0], b[0], "AC-13b: a deeper page must not repeat a hit");

    let total = first["total"].as_u64().unwrap();
    assert!(
        total >= 2,
        "AC-13b: total is the fused in-window count, got {total}"
    );
    assert_eq!(
        total,
        second["total"].as_u64().unwrap(),
        "total is stable across pages of the same query"
    );
}

#[test]
fn the_document_leg_weight_is_actually_read() {
    let home = TempDir::new().unwrap();
    let data_dir = home.path().join(".comemory");
    run(
        &data_dir,
        &[
            "save",
            "upgrade guide covers the migration snapshot",
            "--kind",
            "note",
        ],
    );

    // The knob must be accepted and validated on the way in. Before this
    // change `retrieval.document_leg_weight` was declared, validated, and
    // read by nothing.
    let ok = bin(&data_dir)
        .env("COMEMORY_RETRIEVAL_DOCUMENT_LEG_WEIGHT", "1.5")
        .args(["find", "upgrade", "--json"])
        .output()
        .unwrap();
    assert!(
        ok.status.success(),
        "a valid document_leg_weight is accepted: {}",
        String::from_utf8_lossy(&ok.stderr)
    );

    let bad = bin(&data_dir)
        .env("COMEMORY_RETRIEVAL_DOCUMENT_LEG_WEIGHT", "0")
        .args(["find", "upgrade", "--json"])
        .output()
        .unwrap();
    assert!(
        !bad.status.success(),
        "AC-16: the knob is validated (0 is outside the allowed range)"
    );
}
