#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
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

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;
#[path = "common/vectors.rs"]
mod vectors;

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

/// Issue #153, end to end: a `file:` URL in a memory body used to become a
/// `references_file` edge to a pseudo-repo named `file`, and the bundle
/// then cited `/tmp/check.db` as an unresolvable `unpinned` code ref with
/// a relation to `file:file:/tmp/check.db`. Neither may exist.
#[test]
fn context_does_not_cite_a_file_url_as_a_code_reference() {
    let home = TempDir::new().expect("tempdir");
    bin(&home)
        .args([
            "save",
            "--kind",
            "pattern",
            "--repo",
            "comemory.io",
            r#"Run the gate with TURSO_DATABASE_URL="file:/tmp/check.db" so knip can load drizzle.config.ts."#,
        ])
        .assert()
        .success();

    let v = context_json(&home, "gate", &["--repo", "comemory.io"]);
    let mems = v["memories"].as_array().expect("memories");
    assert_eq!(mems.len(), 1, "the memory itself is surfaced: {v}");
    assert_eq!(
        v["code_refs"].as_array().map(Vec::len),
        Some(0),
        "a file: URL is not a code reference: {v}"
    );
    assert_eq!(
        v["relations"].as_array().map(Vec::len),
        Some(0),
        "no references_file edge was minted for it: {v}"
    );
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

/// Issue #152: every memory row in a bundle reported `score: 0.0`, so a
/// consumer could not tell the top-ranked memory from the last one. The
/// rows must carry the pipeline's `final_score`, non-increasing down the
/// list, exactly as `comemory search --json` ranks the same query.
#[test]
fn context_memory_rows_carry_the_pipeline_score() {
    let home = TempDir::new().expect("tempdir");
    for body in [
        "postgres advisory locks serialize migration ordering across workers",
        "advisory note: the redis lock keeps the session cache warm",
    ] {
        bin(&home)
            .args(["save", "--kind", "decision", "--repo", "foo", body])
            .assert()
            .success();
    }

    // Hold both ranking inputs stable across processes: no access bump and
    // zero time decay, so elapsed wall time cannot change the comparison.
    let out = bin(&home)
        .env("COMEMORY_DISABLE_ACCESS_TRACKING", "true")
        .env("COMEMORY_RANK_DECAY", "0")
        .args(["context", "advisory lock", "--json"])
        .assert()
        .success();
    let v: Value = serde_json::from_slice(&out.get_output().stdout).expect("context json");
    let mems = v
        .get("memories")
        .and_then(Value::as_array)
        .expect("memories");
    assert_eq!(mems.len(), 2, "both memories match lexically: {v}");
    let scores: Vec<f64> = mems
        .iter()
        .map(|m| m["score"].as_f64().expect("score is a number"))
        .collect();
    assert!(
        scores.iter().all(|s| *s > 0.0),
        "every surfaced memory carries a real score, got {scores:?}"
    );
    assert!(
        scores.windows(2).all(|w| w[0] >= w[1]),
        "scores follow the ranked order, got {scores:?}"
    );

    // The bundle's number is the search pipeline's number for the same
    // query — one ranking, two surfaces.
    let out = bin(&home)
        .env("COMEMORY_DISABLE_ACCESS_TRACKING", "true")
        .env("COMEMORY_RANK_DECAY", "0")
        .args(["search", "advisory lock", "--json"])
        .assert()
        .success();
    let search: Value = serde_json::from_slice(&out.get_output().stdout).expect("search json");
    let search_scores: Vec<f64> = search["hits"]
        .as_array()
        .expect("hits")
        .iter()
        .map(|h| h["score"].as_f64().expect("score"))
        .collect();
    assert_eq!(
        scores.len(),
        search_scores.len(),
        "context: {v}\nsearch: {search}"
    );
    assert_eq!(
        scores, search_scores,
        "context scores {scores:?} must match search's {search_scores:?}"
    );
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

/// Index a fixture repo at `<workspace>/code-repo` containing a single
/// `alpha.rs` with two functions, committed and indexed under repo label `r`.
/// Returns the repo path so callers can mutate + recommit it.
fn index_alpha_repo(home: &TempDir, workspace: &TempDir) -> std::path::PathBuf {
    let repo = workspace.path().join("code-repo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(
        &repo,
        &[(
            "alpha.rs",
            "fn alpha_router() {}\nfn unrelated_helper() {}\n",
        )],
        "init",
    );
    bin(home)
        .args(["index-code", "--repo", "r", "--path"])
        .arg(&repo)
        .assert()
        .success();
    repo
}

/// Boost `unrelated_helper` (rank_score + access_count) above `alpha_router`
/// so only the graph priors — not the lexical tie-break — can reorder them.
fn boost_unrelated_helper(home: &TempDir) {
    let conn = connection::open(home.path().join(".comemory").join("comemory.db")).expect("open");
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

/// Read the `access_count` of a `code_symbols` row by symbol name.
fn access_count(db_path: &std::path::Path, symbol: &str) -> i64 {
    let conn = connection::open(db_path).expect("open");
    conn.query_row(
        "SELECT access_count FROM code_symbols WHERE symbol = ?1",
        [symbol],
        |r| r.get(0),
    )
    .expect("access_count row")
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
    index_alpha_repo(&home, &workspace);

    save_memory(
        &home,
        "router decision compares r:alpha.rs:alpha_router and \
         r:alpha.rs:unrelated_helper for dispatch",
        "decision",
    );

    // `alpha_router` wins the (path, symbol) tie-break, so only the priors
    // on `unrelated_helper` can put it on top.
    boost_unrelated_helper(&home);

    let v = context_json(&home, "router decision dispatch", &[]);
    let refs = v["code_refs"].as_array().expect("code_refs array");
    // Two symbol refs (resolved, ranked) plus the file ref `r:alpha.rs` that
    // `cross_link` mines from the same `repo:path:sym` token — file refs newly
    // surface and trail the ranked symbol refs (no rank_parts to sort by).
    assert_eq!(
        refs.len(),
        3,
        "two symbol refs + one file ref expected: {v}"
    );
    assert!(
        refs.iter()
            .any(|r| r["id"].as_str() == Some("r:alpha.rs") && r["symbol"].as_str() == Some("")),
        "the file ref must surface in the bundle: {v}"
    );
    assert_ranked_with_parts(refs);

    // TTY order must match the ranked order.
    assert_tty_ranked_order(&home);
}

/// Assert the boosted symbol leads, carries a full `rank_parts` breakdown, and
/// outranks the second symbol ref by `final_score`.
fn assert_ranked_with_parts(refs: &[Value]) {
    assert_eq!(
        refs[0]["symbol"].as_str(),
        Some("unrelated_helper"),
        "prior-boosted symbol must sort first: {refs:?}"
    );
    for key in ["rank", "activation", "affinity", "feedback", "final_score"] {
        assert!(
            refs[0]["rank_parts"][key].is_number(),
            "rank_parts.{key} missing: {refs:?}"
        );
    }
    let first = refs[0]["rank_parts"]["final_score"]
        .as_f64()
        .expect("score");
    let second = refs[1]["rank_parts"]["final_score"]
        .as_f64()
        .expect("score");
    assert!(first > second, "ranked order must be final_score desc");
}

/// Run `context` in TTY mode and assert the prior-boosted `unrelated_helper`
/// ref renders before `alpha_router`.
fn assert_tty_ranked_order(home: &TempDir) {
    let out = bin(home)
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

/// Phase 0 auto-reinforcement: a tracked `context` lookup must self-reinforce
/// the code refs it actually surfaced — the code-side twin of the memory
/// access bump. End-to-end: index a real fixture repo, save a memory that
/// cross-links one of the two indexed symbols, run `context`, and assert the
/// referenced symbol's `access_count` rose to 1 while the un-referenced
/// symbol stayed at 0 (proving the bump is scoped to returned refs, not the
/// whole index).
#[test]
fn context_bumps_access_count_for_resolved_code_refs() {
    let home = TempDir::new().expect("tempdir");
    let workspace = TempDir::new().expect("workspace");
    index_alpha_repo(&home, &workspace);

    // The memory cross-links ONLY alpha_router, so unrelated_helper never
    // enters the bundle and must keep its zero access count.
    save_memory(
        &home,
        "router decision references r:alpha.rs:alpha_router for dispatch",
        "decision",
    );

    let db_path = home.path().join(".comemory").join("comemory.db");
    // Fresh index: nothing accessed yet.
    assert_eq!(
        access_count(&db_path, "alpha_router"),
        0,
        "fresh index must have zero accessed symbols"
    );

    let v = context_json(&home, "router decision dispatch", &[]);
    let refs = v["code_refs"].as_array().expect("code_refs array");
    assert!(
        refs.iter()
            .any(|r| r["symbol"].as_str() == Some("alpha_router")),
        "alpha_router must be surfaced as a resolved code ref: {v}"
    );

    assert_eq!(
        access_count(&db_path, "alpha_router"),
        1,
        "the resolved code ref must be bumped exactly once by the tracked lookup"
    );
    assert_eq!(
        access_count(&db_path, "unrelated_helper"),
        0,
        "a symbol never surfaced in the bundle must not be bumped"
    );
}

/// Zero pipeline hits: the empty-hits guard skips the working-set build
/// (`WorkingSet::default()` instead of git discovery) and the no-hits
/// context call must still succeed with an empty bundle. The skipped git
/// walk itself is not observable without instrumenting git calls, so this
/// asserts the guarded path's behavior end-to-end.
#[test]
fn context_no_hits_returns_empty_bundle() {
    let home = TempDir::new().expect("tempdir");
    save_memory(&home, "completely unrelated body text", "note");

    let v = context_json(&home, "zzz-no-such-term-zzz", &[]);
    let mems = v
        .get("memories")
        .and_then(Value::as_array)
        .expect("memories");
    assert!(
        mems.is_empty(),
        "no-hits query must yield empty bundle: {v}"
    );
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

#[path = "cli_context/references.rs"]
mod references;
