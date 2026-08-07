#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `comemory consolidate` through the real binary: the JSON envelope, the
//! read-only invariant, flag validation, paging, and the empty corpus.

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

/// Build a `comemory` invocation with `COMEMORY_DATA_DIR` rooted at `home`.
fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
}

/// Save one memory through the binary and return its id.
fn save(home: &TempDir, body: &str, repo: &str) -> String {
    let assertion = bin(home)
        .args(["--json", "save", body, "--kind", "note", "--repo", repo])
        .assert()
        .success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    let v: Value = serde_json::from_str(stdout.trim()).expect("parse save JSON");
    v["id"].as_str().expect("save emits id").to_string()
}

/// Run `consolidate` with `args` and parse the JSON report.
fn report(home: &TempDir, args: &[&str]) -> Value {
    let assertion = bin(home)
        .args(["--json", "consolidate"])
        .args(args)
        .assert()
        .success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    serde_json::from_str(stdout.trim()).expect("parse consolidate JSON")
}

/// Three near-duplicate memories plus one unrelated control.
fn seeded() -> TempDir {
    let home = TempDir::new().expect("tempdir");
    for suffix in ["runner", "worker", "daemon"] {
        save(
            &home,
            &format!("postgres advisory lock ordering fix for the migration {suffix}"),
            "demo",
        );
    }
    save(
        &home,
        "terraform state locking through a dynamodb table",
        "demo",
    );
    home
}

/// Member ids of the first reported cluster.
fn first_cluster(v: &Value) -> Vec<String> {
    v["clusters"]["items"][0]["members"]
        .as_array()
        .expect("cluster has members")
        .iter()
        .map(|m| m["id"].as_str().expect("member id").to_string())
        .collect()
}

#[test]
fn json_reports_the_cluster_envelope_with_keeper_first() {
    let home = seeded();
    let v = report(&home, &[]);
    assert_eq!(
        v["radius"].as_u64(),
        Some(8),
        "defaults to near_dup_hamming"
    );
    assert_eq!(v["scanned"].as_u64(), Some(4));
    assert_eq!(v["skipped_unhashed"].as_u64(), Some(0));
    assert_eq!(v["clusters"]["total"].as_u64(), Some(1));
    assert_eq!(v["clusters"]["has_more"].as_bool(), Some(false));

    let members = first_cluster(&v);
    assert_eq!(
        members.len(),
        3,
        "the three near-duplicates cluster: {members:?}"
    );
    assert_eq!(v["clustered"].as_u64(), Some(3));
    let keeper = &v["clusters"]["items"][0]["members"][0];
    assert_eq!(
        keeper["hamming_to_keeper"].as_u64(),
        Some(0),
        "the keeper anchors its own cluster"
    );
    assert!(
        v["clusters"]["items"][0]["max_hamming"].as_u64().is_some(),
        "every cluster reports its widest distance"
    );
    assert_eq!(keeper["repo"].as_str(), Some("demo"));
}

#[test]
fn the_tty_report_marks_the_keeper_and_offers_the_merge_command() {
    let home = seeded();
    let assertion = bin(&home).arg("consolidate").assert().success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    assert!(stdout.contains("clusters : 1"), "header missing: {stdout}");
    assert!(stdout.contains("← keeper"), "keeper unmarked: {stdout}");
    assert!(
        stdout.contains("merge with: comemory save"),
        "no actionable hint: {stdout}"
    );
    assert!(
        stdout.contains("--supersedes"),
        "the hint must name the members to retire: {stdout}"
    );
}

/// The mutable memory columns, id-ordered — counts alone would miss an
/// in-place rewrite of `access_count`, `quality` or `updated_at`.
fn memory_rows(db: &std::path::Path) -> Vec<(String, i64, i64, String)> {
    let conn = comemory::store::connection::open(db).expect("open mirror");
    let mut stmt = conn
        .prepare("SELECT id, access_count, quality, updated_at FROM memories ORDER BY id")
        .expect("prepare memories snapshot");
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .expect("query memories")
        .collect::<std::result::Result<_, _>>()
        .expect("collect memories")
}

/// Every markdown file's name, size and mtime — a rewrite that kept the file
/// count identical still moves one of the last two.
fn markdown_files(home: &TempDir) -> Vec<(String, u64, std::time::SystemTime)> {
    let mut files: Vec<(String, u64, std::time::SystemTime)> =
        std::fs::read_dir(home.path().join(".comemory").join("memories"))
            .expect("read memories dir")
            .map(|entry| {
                let entry = entry.expect("dir entry");
                let meta = entry.metadata().expect("file metadata");
                (
                    entry.file_name().to_string_lossy().to_string(),
                    meta.len(),
                    meta.modified().expect("mtime"),
                )
            })
            .collect();
    files.sort();
    files
}

/// Rows, markdown, and the edge count in one comparable tuple.
type StoreSnapshot = (
    Vec<(String, i64, i64, String)>,
    Vec<(String, u64, std::time::SystemTime)>,
    i64,
);

/// Capture everything `consolidate` is forbidden from touching.
fn snapshot(home: &TempDir, db: &std::path::Path) -> StoreSnapshot {
    let conn = comemory::store::connection::open(db).expect("open mirror");
    let edges: i64 = conn
        .query_row("SELECT count(*) FROM edges", [], |r| r.get(0))
        .expect("count edges");
    (memory_rows(db), markdown_files(home), edges)
}

#[test]
fn consolidate_never_writes() {
    let home = seeded();
    let db = home.path().join(".comemory").join("comemory.db");
    let before = snapshot(&home, &db);
    let first = report(&home, &[]);
    let second = report(&home, &[]);
    let after = snapshot(&home, &db);
    assert_eq!(
        before.0, after.0,
        "no memory row was touched — access_count, quality and updated_at are unchanged"
    );
    assert_eq!(
        before.1, after.1,
        "no markdown file was created, resized or rewritten"
    );
    assert_eq!(before.2, after.2, "no edge was written");
    assert_eq!(
        first, second,
        "two consecutive runs must produce identical JSON"
    );
}

#[test]
fn radius_zero_clusters_only_identical_bodies() {
    let home = seeded();
    let v = report(&home, &["--radius", "0"]);
    assert_eq!(
        v["clusters"]["total"].as_u64(),
        Some(0),
        "one-word-apart bodies are not fingerprint-identical: {v}"
    );
    assert_eq!(v["radius"].as_u64(), Some(0));
}

#[test]
fn a_radius_above_the_simhash_width_is_a_usage_error() {
    let home = seeded();
    let assertion = bin(&home)
        .args(["consolidate", "--radius", "65"])
        .assert()
        .failure();
    let stderr = String::from_utf8(assertion.get_output().stderr.clone()).expect("utf8 stderr");
    assert!(
        stderr.contains("radius"),
        "the error must name the offending flag: {stderr}"
    );
}

#[test]
fn the_repo_flag_scopes_the_scan() {
    let home = TempDir::new().expect("tempdir");
    save(
        &home,
        "redis cache eviction policy tuning for the session store",
        "alpha",
    );
    save(
        &home,
        "redis cache eviction policy tuning for the session cache",
        "beta",
    );

    let scoped = report(&home, &["--repo", "alpha"]);
    assert_eq!(scoped["scanned"].as_u64(), Some(1));
    assert_eq!(
        scoped["clusters"]["total"].as_u64(),
        Some(0),
        "the twin lives in another repo"
    );

    let corpus_wide = report(&home, &[]);
    assert_eq!(corpus_wide["scanned"].as_u64(), Some(2));
    assert_eq!(
        corpus_wide["clusters"]["total"].as_u64(),
        Some(1),
        "unscoped, the cross-repo pair does cluster"
    );
}

#[test]
fn paging_is_a_window_over_one_stable_ordering() {
    let home = TempDir::new().expect("tempdir");
    for suffix in ["runner", "worker"] {
        save(
            &home,
            &format!("postgres advisory lock ordering fix for the migration {suffix}"),
            "demo",
        );
    }
    for suffix in ["store", "cache"] {
        save(
            &home,
            &format!("redis cache eviction policy tuning for the session {suffix}"),
            "demo",
        );
    }

    let all = report(&home, &[]);
    let total = all["clusters"]["total"].as_u64().expect("total");
    assert!(
        total >= 2,
        "fixture must yield at least two clusters: {all}"
    );

    let first = report(&home, &["--k", "1"]);
    assert_eq!(first["clusters"]["items"].as_array().map(Vec::len), Some(1));
    assert_eq!(first["clusters"]["has_more"].as_bool(), Some(true));
    assert_eq!(first_cluster(&first), first_cluster(&all));

    let second = report(&home, &["--k", "1", "--offset", "1"]);
    assert_eq!(second["clusters"]["offset"].as_u64(), Some(1));
    let expected: Vec<String> = all["clusters"]["items"][1]["members"]
        .as_array()
        .expect("second cluster")
        .iter()
        .map(|m| m["id"].as_str().expect("member id").to_string())
        .collect();
    assert_eq!(
        first_cluster(&second),
        expected,
        "offset 1 returns the second cluster of the unpaged list"
    );
    assert_eq!(second["clusters"]["total"].as_u64(), Some(total));
}

#[test]
fn an_empty_corpus_reports_zero_clusters_and_exits_clean() {
    let home = TempDir::new().expect("tempdir");
    let v = report(&home, &[]);
    assert_eq!(v["clusters"]["total"].as_u64(), Some(0));
    assert_eq!(v["scanned"].as_u64(), Some(0));
    assert_eq!(v["clustered"].as_u64(), Some(0));
    assert!(
        v["clusters"]["items"]
            .as_array()
            .expect("items array")
            .is_empty()
    );

    let assertion = bin(&home).arg("consolidate").assert().success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).expect("utf8 stdout");
    assert!(
        stdout.contains("clusters : 0"),
        "TTY must state the empty result rather than print nothing: {stdout}"
    );
}
