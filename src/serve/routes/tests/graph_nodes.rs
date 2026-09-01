#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! End-to-end coverage of the console-api §5 graph routes, driven through the
//! REAL router (`tests/common/serve_state.rs`, `tower::ServiceExt::oneshot`)
//! over a REAL indexed git repo seeded into the session's data-dir.
//!
//! Also pins the `{id}` transport contract: a node id is
//! `file:<repo>:<path>`, whose path half contains `/`, so a client sends it
//! as ONE percent-encoded segment (`file%3Ademo%3Asrc%2Fa.rs`).

use crate::test_common::serve_state::{self, Session};

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::store::connection;
use serde_json::Value;

use crate::test_common::{git_commit, git_repo};
use tempfile::TempDir;

/// Repo label every test in this file indexes under.
const REPO: &str = "demo";

/// The percent-encoded `file:demo:src/a.rs` node id, as a console sends it.
const NODE_A: &str = "file%3Ademo%3Asrc%2Fa.rs";

/// Build `<root>/import-repo`: `src/a.rs` declares `mod b;`, minting a real
/// `imports` edge between two indexed files.
fn build_import_repo(root: &Path) -> PathBuf {
    let repo = root.join("import-repo");
    git_repo::init_repo(&repo);
    git_commit::commit_files(
        &repo,
        &[
            (
                "src/a.rs",
                "mod b;\n\npub fn alpha() {\n    b::beta();\n}\n",
            ),
            ("src/b.rs", "pub fn beta() {}\n"),
        ],
        "seed a + b",
    );
    repo
}

/// Index a fresh repo into `session`'s data-dir through the real command
/// core, over its own short-lived connection (the router's is private), and
/// return the workspace tempdir so the checkout outlives the test body.
fn seed_indexed_repo(session: &Session) -> TempDir {
    let workspace = TempDir::new().expect("workspace");
    let repo_root = build_import_repo(workspace.path());
    let paths = Paths::new(session.home.path());
    let cfg = Config::defaults();
    let mut conn = connection::open(paths.db_path()).expect("open db for seed index");
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    api::index_code::run(
        &mut ctx,
        api::index_code::Request {
            repo: REPO.to_string(),
            path: repo_root.to_str().expect("utf8 repo path").to_string(),
            mode: api::index_code::IndexMode::Incremental,
        },
    )
    .expect("seed index_code run");
    workspace
}

/// Every `(path, symbol, rank_score)` triple for `REPO`, read over a fresh
/// connection to `session`'s database.
fn ranks(session: &Session) -> Vec<(String, String, f64)> {
    let paths = Paths::new(session.home.path());
    let conn = connection::open(paths.db_path()).expect("open db for ranks");
    let mut stmt = conn
        .prepare(
            "SELECT path, symbol, rank_score FROM code_symbols \
              WHERE repo = ?1 ORDER BY path, symbol",
        )
        .expect("prepare ranks");
    stmt.query_map([REPO], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("query ranks")
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("collect ranks")
}

/// Poll `GET /api/v1/jobs/{id}` until it reports a terminal status,
/// returning the envelope's `data` object.
async fn poll_job_terminal(session: &Session, job_id: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let res = serve_state::send(session, "GET", &format!("/api/v1/jobs/{job_id}"), None).await;
        let data = res.json["data"].clone();
        if matches!(
            data["status"].as_str(),
            Some("done" | "error" | "cancelled")
        ) {
            return data;
        }
        assert!(
            Instant::now() < deadline,
            "job {job_id} never reached a terminal status: {}",
            res.text
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn graph_nodes_lists_the_indexed_files() {
    let session = serve_state::session(false);
    let _workspace = seed_indexed_repo(&session);

    let res = serve_state::send(&session, "GET", "/api/v1/graph/nodes?repo=demo", None).await;
    assert_eq!(res.status, 200, "body: {}", res.text);
    let items = res.json["data"]["items"].as_array().expect("items");
    assert_eq!(items.len(), 2, "body: {}", res.text);
    let ids: Vec<&str> = items.iter().filter_map(|n| n["id"].as_str()).collect();
    assert!(ids.contains(&"file:demo:src/a.rs"), "ids: {ids:?}");
    assert_eq!(res.json["data"]["total"].as_u64(), Some(2));
}

#[tokio::test]
async fn graph_nodes_rejects_an_unknown_sort_with_400() {
    let session = serve_state::session(false);
    let _workspace = seed_indexed_repo(&session);

    let res = serve_state::send(&session, "GET", "/api/v1/graph/nodes?sort=rank", None).await;
    assert_eq!(res.status, 400, "body: {}", res.text);
    assert_eq!(res.json["error"]["code"], "bad_request");
}

#[tokio::test]
async fn graph_node_detail_takes_one_percent_encoded_segment() {
    let session = serve_state::session(false);
    let _workspace = seed_indexed_repo(&session);

    let res = serve_state::send(
        &session,
        "GET",
        &format!("/api/v1/graph/nodes/{NODE_A}"),
        None,
    )
    .await;
    assert_eq!(res.status, 200, "body: {}", res.text);
    assert_eq!(res.json["data"]["node"]["id"], "file:demo:src/a.rs");
    let symbols = res.json["data"]["top_symbols"]
        .as_array()
        .expect("top_symbols");
    assert!(
        symbols.iter().any(|s| s["symbol"] == "alpha"),
        "body: {}",
        res.text
    );
    assert!(res.json["data"]["cited_by"].is_array());
}

#[tokio::test]
async fn graph_node_detail_is_404_for_an_unindexed_file() {
    let session = serve_state::session(false);
    let _workspace = seed_indexed_repo(&session);

    let res = serve_state::send(
        &session,
        "GET",
        "/api/v1/graph/nodes/file%3Ademo%3Asrc%2Fmissing.rs",
        None,
    )
    .await;
    assert_eq!(res.status, 404, "body: {}", res.text);
    assert_eq!(res.json["error"]["code"], "not_found");
}

#[tokio::test]
async fn graph_node_detail_resolves_a_bare_path_under_the_repo_header_scope() {
    let session = serve_state::session(false);
    let _workspace = seed_indexed_repo(&session);

    let res = serve_state::send_headers(
        &session,
        "GET",
        "/api/v1/graph/nodes/src%2Fa.rs",
        &[
            ("Host", "127.0.0.1"),
            ("X-Comemory-Token", &session.token),
            ("X-Comemory-Repo", REPO),
        ],
        None,
    )
    .await;
    assert_eq!(res.status, 200, "body: {}", res.text);
    assert_eq!(res.json["data"]["node"]["id"], "file:demo:src/a.rs");
}

#[tokio::test]
async fn graph_node_neighbors_lists_the_imports_counterpart() {
    let session = serve_state::session(false);
    let _workspace = seed_indexed_repo(&session);

    let res = serve_state::send(
        &session,
        "GET",
        &format!("/api/v1/graph/nodes/{NODE_A}/neighbors"),
        None,
    )
    .await;
    assert_eq!(res.status, 200, "body: {}", res.text);
    let rows = res.json["data"].as_array().expect("neighbor rows");
    assert!(
        rows.iter()
            .any(|r| r["rel"] == "imports" && r["path"] == "src/b.rs" && r["weight"] == 1),
        "body: {}",
        res.text
    );
}

#[tokio::test]
async fn graph_snapshot_reports_an_untruncated_small_graph() {
    let session = serve_state::session(false);
    let _workspace = seed_indexed_repo(&session);

    let res = serve_state::send(&session, "GET", "/api/v1/graph/snapshot?repo=demo", None).await;
    assert_eq!(res.status, 200, "body: {}", res.text);
    let edges = res.json["data"]["edges"].as_array().expect("edges");
    assert!(!edges.is_empty(), "body: {}", res.text);
    assert_eq!(res.json["data"]["truncated"], false);
    assert_eq!(
        res.json["data"]["total_edges"].as_u64(),
        Some(edges.len() as u64)
    );
}

#[tokio::test]
async fn graph_recompute_runs_as_a_job_and_leaves_an_unchanged_graph_identical() {
    let session = serve_state::session(false);
    let _workspace = seed_indexed_repo(&session);
    let before = ranks(&session);
    assert!(
        before.iter().any(|(_, _, score)| *score > 0.0),
        "the seed index must have materialized a real PageRank: {before:?}"
    );

    let res = serve_state::send(&session, "POST", "/api/v1/graph/recompute", None).await;
    assert_eq!(res.status, 202, "body: {}", res.text);
    let job_id = res.json["data"]["job_id"]
        .as_str()
        .expect("job_id")
        .to_string();

    let job = poll_job_terminal(&session, &job_id).await;
    assert_eq!(job["status"], "done", "job: {job}");
    assert!(
        job["result"]["symbols_scored"].as_u64().unwrap_or(0) > 0,
        "job: {job}"
    );
    assert_eq!(job["result"]["repos"][0], REPO);
    assert_eq!(
        before,
        ranks(&session),
        "a deterministic recompute of an unchanged graph rewrites the same scores"
    );
}

#[tokio::test]
async fn graph_recompute_is_405_on_a_read_only_server() {
    let session = serve_state::session(true);

    let res = serve_state::send(&session, "POST", "/api/v1/graph/recompute", None).await;
    assert_eq!(res.status, 405, "body: {}", res.text);
    assert_eq!(res.json["error"]["code"], "read_only");
}
