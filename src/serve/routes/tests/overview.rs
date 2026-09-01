#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `GET /api/v1/overview` and `GET /api/v1/overview/eval-series` driven
//! through the real router (`tests/common/serve_state.rs`) against a real
//! seeded store — console-api spec AC-3 at the transport layer.

use comemory::config::Paths;
use comemory::memory::Kind;
use comemory::store::{connection, eval_runs};

use crate::test_common::serve_state::{self, Session};

/// A second, short-lived connection onto the session's database — the
/// router's own connection is private, so history rows are seeded (and
/// counters cross-checked) through this.
fn conn(session: &Session) -> rusqlite::Connection {
    let paths = Paths::new(session.home.path());
    connection::open(paths.db_path()).expect("open db")
}

fn seed_eval_run(conn: &rusqlite::Connection, id: &str, at: &str, recall: f64) {
    eval_runs::insert(
        conn,
        &eval_runs::NewRun {
            id,
            kind: "eval",
            at,
            golden_pairs: 8,
            k: 5,
            recall,
            mrr: 0.5,
            knobs: "{}",
            applied: false,
        },
    )
    .expect("insert eval run");
}

#[tokio::test]
async fn overview_reports_the_live_corpus_and_no_run_history_yet() {
    let session = serve_state::session(false);
    serve_state::save(&session, "first console memory", Kind::Note, "app");
    serve_state::save(&session, "second console memory", Kind::Decision, "app");
    serve_state::save(&session, "third console memory", Kind::Bug, "app");

    let resp = serve_state::send(&session, "GET", "/api/v1/overview", None).await;

    assert_eq!(resp.status, 200, "body: {}", resp.text);
    assert_eq!(resp.json["ok"], true);
    assert_eq!(resp.json["meta"]["command"], "overview");
    let data = &resp.json["data"];

    let live: i64 = conn(&session)
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE deleted_at IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(data["counters"]["memories"], live);
    assert_eq!(data["counters"]["memories"], 3);
    assert_eq!(data["counters"]["code_symbols"], 0);
    assert!(data["counters"]["db_bytes"].as_u64().unwrap() > 0);

    let recent = data["recent_memories"].as_array().unwrap();
    assert!(recent.len() <= 4, "the Overview list shows at most four");
    assert_eq!(recent.len(), 3);

    assert!(data["last_run"].is_null(), "nothing has been indexed");
    assert!(data["metrics"].is_null());
    assert_eq!(data["eval_series"].as_array().unwrap().len(), 0);
    assert_eq!(data["index_state"]["status"], "unknown");
    assert!(
        data["index_state"]["checked_at"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "the freshness probe stamps when it ran"
    );
}

#[tokio::test]
async fn the_repo_header_scopes_the_overview_counters() {
    let session = serve_state::session(false);
    serve_state::save(&session, "memory filed under alpha", Kind::Note, "alpha");
    serve_state::save(&session, "memory filed under beta", Kind::Note, "beta");

    let token = session.token.clone();
    let scoped = serve_state::send_headers(
        &session,
        "GET",
        "/api/v1/overview",
        &[
            ("Host", "127.0.0.1"),
            ("X-Comemory-Token", &token),
            ("X-Comemory-Repo", "alpha"),
        ],
        None,
    )
    .await;

    assert_eq!(scoped.status, 200, "body: {}", scoped.text);
    assert_eq!(scoped.json["data"]["counters"]["memories"], 1);
    assert_eq!(
        scoped.json["data"]["recent_memories"][0]["repo"], "alpha",
        "the header narrowed the recent list too"
    );

    let explicit = serve_state::send_headers(
        &session,
        "GET",
        "/api/v1/overview?repo=beta",
        &[
            ("Host", "127.0.0.1"),
            ("X-Comemory-Token", &token),
            ("X-Comemory-Repo", "alpha"),
        ],
        None,
    )
    .await;
    assert_eq!(explicit.json["data"]["recent_memories"][0]["repo"], "beta");
}

#[tokio::test]
async fn metrics_and_the_series_surface_real_eval_history() {
    let session = serve_state::session(false);
    {
        let db = conn(&session);
        seed_eval_run(&db, "aaaaaaaaaaaaaaaa", "2026-08-01T00:00:00Z", 0.30);
        seed_eval_run(&db, "bbbbbbbbbbbbbbbb", "2026-08-02T00:00:00Z", 0.44);
    }

    let resp = serve_state::send(&session, "GET", "/api/v1/overview", None).await;

    assert_eq!(resp.status, 200, "body: {}", resp.text);
    assert_eq!(resp.json["data"]["metrics"]["recall_at_k"], 0.44);
    assert_eq!(resp.json["data"]["metrics"]["at"], "2026-08-02T00:00:00Z");
    assert_eq!(resp.json["data"]["metrics"]["golden_queries"], 8);
    let series = resp.json["data"]["eval_series"].as_array().unwrap();
    assert_eq!(series.len(), 2);
    assert_eq!(series[0]["at"], "2026-08-01T00:00:00Z", "oldest first");
    assert_eq!(series[1]["at"], "2026-08-02T00:00:00Z");
}

#[tokio::test]
async fn the_eval_series_route_honors_its_limit_and_stays_oldest_first() {
    let session = serve_state::session(false);
    {
        let db = conn(&session);
        for day in 1..=4 {
            seed_eval_run(
                &db,
                &format!("{day:016x}"),
                &format!("2026-08-0{day}T00:00:00Z"),
                f64::from(day) / 10.0,
            );
        }
    }

    let all = serve_state::send(&session, "GET", "/api/v1/overview/eval-series", None).await;
    assert_eq!(all.status, 200, "body: {}", all.text);
    assert_eq!(all.json["meta"]["command"], "overview.eval-series");
    assert_eq!(all.json["data"].as_array().unwrap().len(), 4);

    let capped = serve_state::send(
        &session,
        "GET",
        "/api/v1/overview/eval-series?limit=2",
        None,
    )
    .await;
    let points = capped.json["data"].as_array().unwrap();
    assert_eq!(points.len(), 2);
    assert_eq!(points[0]["at"], "2026-08-03T00:00:00Z");
    assert_eq!(points[1]["at"], "2026-08-04T00:00:00Z");
    assert_eq!(points[1]["recall_at_k"], 0.4);
}

#[tokio::test]
async fn both_overview_routes_are_readable_on_a_read_only_server() {
    let session = serve_state::session(true);

    let overview = serve_state::send(&session, "GET", "/api/v1/overview", None).await;
    let series = serve_state::send(&session, "GET", "/api/v1/overview/eval-series", None).await;

    assert_eq!(overview.status, 200, "body: {}", overview.text);
    assert_eq!(series.status, 200, "body: {}", series.text);
}
