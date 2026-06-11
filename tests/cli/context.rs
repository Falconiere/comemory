//! Integration tests for `comemory context`.
//!
//! Covers:
//! - Lexical-only path (no vector).
//! - Vector path (--vector-stdin JSON, 1024-dim).
//! - Deep relation walk: supersedes chain surfaced in bundle relations.

use assert_cmd::Command;
use comemory::store::connection;
use serde_json::Value;
use tempfile::TempDir;

use super::{git_setup, vectors};

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

/// Insert a deterministic non-zero vector row for `id` into `memory_vec`.
/// Uses the vectors helper so the vector is well-scaled (no zero components),
/// which is required for cosine distance to be well-defined.
fn seed_unit_vector(home: &TempDir, id: &str, dim: usize) {
    let data_dir = home.path().join(".comemory");
    let conn = connection::open(data_dir.join("comemory.db")).expect("open");
    let v = vectors::vector(id, dim);
    // Encode via the same LE-float32 BLOB path the live INSERT path uses.
    let blob: Vec<u8> = v.iter().flat_map(|f| f.to_le_bytes()).collect();
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

/// --vector-stdin path: 1024-dim non-zero vector triggers ANN branch; bundle shape valid.
/// Cosine distance requires non-zero vectors; we use the deterministic helper
/// so the query and the stored vector are both well-formed unit-scale vectors.
/// Uses --vector-stdin rather than --vector CSV to avoid clap misinterpreting
/// a CSV string that starts with a negative float as a flag.
#[test]
fn context_vector_path_accepts_stdin_vector() {
    let home = TempDir::new().expect("tempdir");
    let id = save_memory(&home, "vector path context body", "note");
    seed_unit_vector(&home, &id, 1024);
    let query_vec = vectors::vector("context-query", 1024);
    let payload = serde_json::to_string(&serde_json::json!({ "embedding": query_vec }))
        .expect("json payload");
    let out = bin(&home)
        .args(["context", "vector path", "--json", "--vector-stdin"])
        .write_stdin(payload)
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    let v: Value = serde_json::from_str(&stdout).expect("json");
    assert!(v.get("memories").and_then(Value::as_array).is_some());
}

/// M2 final-integration review (finding H): `comemory context` runs the
/// tracked pipeline, so it must surface the `query_id` of its retrieval_log
/// row — otherwise context lookups enter the log but can never receive
/// feedback, and `mine` permanently counts every one as a failed query.
#[test]
fn context_json_emits_query_id_and_logs_retrieval() {
    let home = TempDir::new().expect("tempdir");
    save_memory(&home, "postgres advisory locks for ordering", "note");

    let v = context_json(&home, "advisory locks", &[]);
    let qid = v
        .get("query_id")
        .and_then(Value::as_str)
        .expect("context envelope must carry query_id");
    assert!(
        qid.starts_with("q-") && qid.len() == "q-20260611-a1b2c3d4".len(),
        "query_id must have the q-<yyyymmdd>-<8hex> shape: {qid:?}"
    );

    let conn = connection::open(home.path().join(".comemory").join("comemory.db")).expect("open");
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM retrieval_log WHERE query_id = ?1",
            [qid],
            |r| r.get(0),
        )
        .expect("count retrieval_log");
    assert_eq!(n, 1, "the emitted query_id must join retrieval_log");
}

/// TTY mode must print the same `query: <qid>` footer as `comemory search`
/// (with the feedback hint, since the lookup produced memory hits).
#[test]
fn context_tty_prints_query_footer() {
    let home = TempDir::new().expect("tempdir");
    save_memory(&home, "tty footer context body", "note");

    let out = bin(&home)
        .args(["context", "tty footer"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(
        stdout.contains("query: q-"),
        "TTY context must print the query id footer: {stdout:?}"
    );
    assert!(
        stdout.contains("feedback:"),
        "TTY context with hits must print the feedback hint: {stdout:?}"
    );
}

/// Task 14: code refs in the context bundle are ranked by the four graph
/// priors. End-to-end: index a real fixture repo, save a memory whose body
/// cross-links both symbols, boost the alphabetically-later symbol's
/// rank_score + access_count, and assert it sorts first in both JSON
/// (with serialized `rank_parts`) and TTY output.
#[test]
fn context_code_refs_ranked_by_priors_with_rank_parts() {
    let home = TempDir::new().expect("tempdir");
    let workspace = TempDir::new().expect("workspace");
    let repo = workspace.path().join("code-repo");
    git_setup::init_repo(&repo);
    git_setup::commit_files(
        &repo,
        &[(
            "alpha.rs",
            "fn alpha_router() {}\nfn unrelated_helper() {}\n",
        )],
        "init",
    );
    bin(&home)
        .args(["index-code", "--repo", "r", "--path"])
        .arg(&repo)
        .assert()
        .success();

    save_memory(
        &home,
        "router decision compares r:alpha.rs:alpha_router and \
         r:alpha.rs:unrelated_helper for dispatch",
        "decision",
    );

    // `alpha_router` wins the (path, symbol) tie-break, so only the priors
    // on `unrelated_helper` can put it on top.
    {
        let conn =
            connection::open(home.path().join(".comemory").join("comemory.db")).expect("open");
        conn.execute(
            "UPDATE code_symbols SET rank_score = 0.9, access_count = 30 \
             WHERE symbol = 'unrelated_helper'",
            [],
        )
        .expect("boost unrelated_helper");
        conn.execute(
            "UPDATE code_symbols SET rank_score = 0.1 WHERE symbol = 'alpha_router'",
            [],
        )
        .expect("set alpha_router rank");
    }

    let v = context_json(&home, "router decision dispatch", &[]);
    let refs = v["code_refs"].as_array().expect("code_refs array");
    assert_eq!(refs.len(), 2, "both referenced symbols expected: {v}");
    assert_eq!(
        refs[0]["symbol"].as_str(),
        Some("unrelated_helper"),
        "prior-boosted symbol must sort first: {v}"
    );
    for key in ["rank", "activation", "affinity", "feedback", "final_score"] {
        assert!(
            refs[0]["rank_parts"][key].is_number(),
            "rank_parts.{key} missing: {v}"
        );
    }
    let first = refs[0]["rank_parts"]["final_score"]
        .as_f64()
        .expect("final_score");
    let second = refs[1]["rank_parts"]["final_score"]
        .as_f64()
        .expect("final_score");
    assert!(first > second, "ranked order must be final_score desc: {v}");

    // TTY order must match the ranked order.
    let out = bin(&home)
        .args(["context", "router decision dispatch"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    let hot = stdout
        .find("r:alpha.rs:unrelated_helper")
        .unwrap_or_else(|| panic!("boosted ref missing from TTY output: {stdout}"));
    let cold = stdout
        .find("r:alpha.rs:alpha_router")
        .unwrap_or_else(|| panic!("other ref missing from TTY output: {stdout}"));
    assert!(hot < cold, "TTY order must match ranked order: {stdout}");
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
