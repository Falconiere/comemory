//! Integration tests for `comemory context`.
//!
//! Covers:
//! - Lexical-only path (no vector).
//! - Vector path (--vector CSV, 1024-dim).
//! - Deep relation walk: supersedes chain surfaced in bundle relations.

use assert_cmd::Command;
use comemory::store::connection;
use serde_json::Value;
use tempfile::TempDir;

fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
}

fn extract_saved_id(stdout: &str) -> String {
    stdout
        .lines()
        .find(|l| l.starts_with("saved "))
        .expect("save stdout has 'saved <id>' line")
        .strip_prefix("saved ")
        .expect("strip prefix")
        .split_whitespace()
        .next()
        .expect("id token")
        .to_string()
}

/// Save a memory, return its id.
fn save_memory(home: &TempDir, body: &str, kind: &str) -> String {
    let out = bin(home)
        .args(["save", body, "--kind", kind])
        .assert()
        .success();
    extract_saved_id(&String::from_utf8(out.get_output().stdout.clone()).expect("utf8"))
}

/// Insert a zero-blob vector row for `id` into `memory_vec`.
fn seed_zero_vector(home: &TempDir, id: &str, dim: usize) {
    let data_dir = home.path().join(".comemory");
    let conn = connection::open(data_dir.join("comemory.db")).expect("open");
    let blob: Vec<u8> = vec![0u8; dim * 4];
    conn.execute(
        "INSERT OR REPLACE INTO memory_vec(memory_id, embedding) VALUES(?1, ?2)",
        rusqlite::params![id, blob],
    )
    .expect("insert vector");
}

/// Insert a `supersedes` edge between two memory ids.
fn seed_supersedes_edge(home: &TempDir, src: &str, dst: &str) {
    let data_dir = home.path().join(".comemory");
    let conn = connection::open(data_dir.join("comemory.db")).expect("open");
    conn.execute(
        "INSERT OR IGNORE INTO edges(src_kind,src_id,dst_kind,dst_id,rel,created_at) \
         VALUES('memory',?1,'memory',?2,'supersedes',strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        rusqlite::params![src, dst],
    )
    .expect("insert edge");
}

/// Run `comemory context <query> --json` and parse the JSON bundle.
fn context_json(home: &TempDir, query: &str, extra_args: &[&str]) -> Value {
    let mut args = vec!["context", query, "--json"];
    args.extend_from_slice(extra_args);
    let out = bin(home).args(&args).assert().success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    serde_json::from_str(&stdout).expect("json")
}

#[test]
fn context_returns_bundle_for_seeded_memory() {
    let home = TempDir::new().expect("tempdir");
    bin(&home)
        .args([
            "save",
            "--kind",
            "decision",
            "--repo",
            "foo",
            "postgres advisory locks for migration ordering",
        ])
        .assert()
        .success();

    let v = context_json(&home, "advisory lock", &[]);
    assert_eq!(
        v.get("query").and_then(Value::as_str),
        Some("advisory lock")
    );
    let mems = v
        .get("memories")
        .and_then(Value::as_array)
        .expect("memories");
    assert!(!mems.is_empty());
}

/// Lexical-only (no --vector): bundle must come back without error.
#[test]
fn context_lexical_path_no_vector() {
    let home = TempDir::new().expect("tempdir");
    save_memory(&home, "lexical only context body", "note");
    let v = context_json(&home, "lexical only", &[]);
    assert!(v.get("memories").and_then(Value::as_array).is_some());
}

/// --vector path: 1024-dim zero vector triggers ANN branch; bundle shape valid.
#[test]
fn context_vector_path_accepts_csv_vector() {
    let home = TempDir::new().expect("tempdir");
    let id = save_memory(&home, "vector path context body", "note");
    seed_zero_vector(&home, &id, 1024);
    let vec_csv = vec!["0.0"; 1024].join(",");
    let v = context_json(&home, "vector path", &["--vector", &vec_csv]);
    assert!(v.get("memories").and_then(Value::as_array).is_some());
}

/// Supersedes chain: bundle relations must include the supersedes edge.
#[test]
fn context_bundle_includes_supersedes_relations() {
    let home = TempDir::new().expect("tempdir");
    let id1 = save_memory(&home, "old decision body supersedes chain", "decision");
    let id2 = save_memory(&home, "new decision body supersedes chain", "decision");
    seed_supersedes_edge(&home, &id1, &id2);

    let v = context_json(&home, "old decision body supersedes", &[]);
    let rels = v
        .get("relations")
        .and_then(Value::as_array)
        .expect("relations");
    assert!(
        rels.iter()
            .any(|r| r.get("rel").and_then(Value::as_str) == Some("supersedes")),
        "expected supersedes in relations; got: {v}"
    );
}
