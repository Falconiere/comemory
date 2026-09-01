#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The `/api/v1/learning/*` console surface driven through the real router
//! (`tests/common/serve_state.rs`), plus the `POST /learning/evals` alias
//! driven through the real binary (`tests/common/serve_learning_support.rs`)
//! because it is a job route. Covers console-api spec AC-13 at the
//! transport layer: a proposal is listed with `from != to`, `apply`
//! (confirmed) rewrites `config.toml` and retires it, `discard` retires
//! another without touching the file.

use comemory::config::Paths;
use comemory::eval::tune::TuneCandidate;
use comemory::memory::Kind;
use comemory::store::{connection, eval_runs};
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::test_common::serve_learning_support as support;
use crate::test_common::serve_state::{self, Session};

/// A second, short-lived connection onto the session's database — the
/// router's own connection is private, so history rows are seeded through
/// this one.
fn conn(session: &Session) -> rusqlite::Connection {
    let paths = Paths::new(session.home.path());
    connection::open(paths.db_path()).expect("open db")
}

/// A knob set differing from `Config::defaults()` on `rrf_k` alone.
fn shifted() -> TuneCandidate {
    let base = comemory::config::Config::defaults();
    TuneCandidate {
        rrf_k: 25.0,
        decay: base.rank.decay,
        mmr_lambda: base.rank.mmr_lambda,
        bm25_weights: base.retrieval.bm25_weights,
        graph_hops: base.retrieval.graph_hops,
        graph_seeds: base.retrieval.graph_seeds,
    }
}

/// Insert one run row of the given kind, carrying `knobs` and `recall`.
fn seed_run(conn: &rusqlite::Connection, id: &str, kind: &str, at: &str, recall: f64) {
    eval_runs::insert(
        conn,
        &eval_runs::NewRun {
            id,
            kind,
            at,
            golden_pairs: 12,
            k: 3,
            recall,
            mrr: 0.4,
            knobs: "{}",
            applied: false,
        },
    )
    .expect("insert eval_runs row");
}

/// A run row carrying a full, live-config-differing knob set.
fn seed_proposal(conn: &rusqlite::Connection, id: &str, at: &str) {
    let knobs = serde_json::to_string(&shifted()).expect("serialize knobs");
    eval_runs::insert(
        conn,
        &eval_runs::NewRun {
            id,
            kind: "tune",
            at,
            golden_pairs: 12,
            k: 3,
            recall: 0.6,
            mrr: 0.4,
            knobs: &knobs,
            applied: false,
        },
    )
    .expect("insert eval_runs row");
}

#[tokio::test]
async fn summary_and_evals_expose_the_seeded_run_history() {
    let session = serve_state::session(false);
    {
        let c = conn(&session);
        seed_run(&c, "r1", "eval", "2026-08-30T10:00:00.000000000Z", 0.40);
        seed_run(&c, "r2", "tune", "2026-08-31T10:00:00.000000000Z", 0.60);
        seed_run(&c, "r3", "eval", "2026-09-01T10:00:00.000000000Z", 0.50);
    }

    let resp = serve_state::send(&session, "GET", "/api/v1/learning/summary", None).await;
    assert_eq!(resp.status, 200, "body: {}", resp.text);
    assert_eq!(resp.json["meta"]["command"], "learning.summary");
    let data = &resp.json["data"];
    assert_eq!(data["latest"]["id"], "r3");
    assert_eq!(data["latest"]["recall_at_k"].as_f64(), Some(0.50));
    assert_eq!(data["feedback_events"].as_u64(), Some(0));
    assert_eq!(data["implicit_share"].as_f64(), Some(0.0));
    assert_eq!(data["expansions"].as_u64(), Some(0));
    // r2 (0.60, tune) against r1 (0.40, the nearest earlier eval).
    assert!(
        (data["best_delta"].as_f64().expect("best_delta") - 0.20).abs() < 1e-9,
        "got {}",
        data["best_delta"]
    );

    let resp = serve_state::send(&session, "GET", "/api/v1/learning/evals?limit=2", None).await;
    assert_eq!(resp.status, 200, "body: {}", resp.text);
    let rows = resp.json["data"].as_array().expect("an array of runs");
    assert_eq!(rows.len(), 2, "?limit= caps the page");
    assert_eq!(rows[0]["id"], "r3");
    assert_eq!(rows[0]["is_baseline"], true);
    assert!((rows[0]["delta"].as_f64().expect("delta") + 0.10).abs() < 1e-9);
    assert_eq!(rows[1]["id"], "r2");
    assert_eq!(rows[1]["is_best"], true, "0.60 leads the returned page");
    assert_eq!(rows[1]["is_baseline"], false);
}

#[tokio::test]
async fn golden_set_returns_the_real_feedback_harvest() {
    let session = serve_state::session(false);
    let query = "sqlite wal checkpoint starvation under sustained writes";
    serve_state::save(&session, query, Kind::Note, "app");
    let id: String = conn(&session)
        .query_row("SELECT id FROM memories", [], |r| r.get(0))
        .expect("the saved memory id");

    let search = serve_state::send(
        &session,
        "POST",
        "/api/v1/memories/search",
        Some(json!({ "query": query })),
    )
    .await;
    assert_eq!(search.status, 200, "body: {}", search.text);
    let query_id = search.json["data"]["query_id"]
        .as_str()
        .expect("a tracked search records a query id")
        .to_string();

    let feedback = serve_state::send(
        &session,
        "POST",
        "/api/v1/feedback",
        Some(json!({ "query_id": query_id, "used": [id.clone()] })),
    )
    .await;
    assert_eq!(feedback.status, 200, "body: {}", feedback.text);

    let resp = serve_state::send(&session, "GET", "/api/v1/learning/golden-set", None).await;
    assert_eq!(resp.status, 200, "body: {}", resp.text);
    assert_eq!(resp.json["meta"]["command"], "learning.golden-set");
    let data = &resp.json["data"];
    assert_eq!(data["harvested"].as_u64(), Some(1));
    assert_eq!(data["from_file"].as_u64(), Some(0));
    assert_eq!(data["count"].as_u64(), Some(1));
    assert_eq!(data["pairs"][0]["query"], query);
    assert_eq!(data["pairs"][0]["relevant"][0], id);
}

#[tokio::test]
async fn golden_set_refuses_a_file_outside_every_allowed_root() {
    let session = serve_state::session(false);
    let outside = TempDir::new().expect("outside dir");
    let golden = outside.path().join("golden.yaml");
    std::fs::write(&golden, "- query: q\n  relevant: [aaaaaaaa]\n").expect("write golden");

    let path = golden.to_str().expect("utf8 path");
    let resp = serve_state::send(
        &session,
        "GET",
        &format!("/api/v1/learning/golden-set?golden={path}"),
        None,
    )
    .await;
    assert_eq!(resp.status, 403, "body: {}", resp.text);
    assert_eq!(resp.json["error"]["code"], "forbidden");
}

#[tokio::test]
async fn expansions_pages_the_mined_table() {
    let session = serve_state::session(false);
    {
        let c = conn(&session);
        for (term, expansion, support) in [("wal", "checkpoint", 9_i64), ("lock", "advisory", 5)] {
            c.execute(
                "INSERT INTO query_expansions(term, expansion, support, last_mined) \
                 VALUES (?1, ?2, ?3, '2026-09-01T00:00:00.000000000Z')",
                rusqlite::params![term, expansion, support],
            )
            .expect("insert expansion");
        }
    }

    let resp = serve_state::send(
        &session,
        "GET",
        "/api/v1/learning/expansions?limit=1&offset=0",
        None,
    )
    .await;
    assert_eq!(resp.status, 200, "body: {}", resp.text);
    let data = &resp.json["data"];
    assert_eq!(data["total"].as_u64(), Some(2));
    assert_eq!(data["has_more"], true);
    assert_eq!(data["items"][0]["from"], "wal");
    assert_eq!(data["items"][0]["to"], "checkpoint");
    assert_eq!(data["items"][0]["count"].as_u64(), Some(9));

    let resp = serve_state::send(
        &session,
        "GET",
        "/api/v1/learning/expansions?limit=1&offset=1",
        None,
    )
    .await;
    let data = &resp.json["data"];
    assert_eq!(data["items"][0]["from"], "lock");
    assert_eq!(data["has_more"], false);
}

#[tokio::test]
async fn apply_needs_confirm_then_rewrites_config_and_retires_the_proposal_ac13() {
    let session = serve_state::session(false);
    seed_proposal(&conn(&session), "prop1", "2026-09-01T10:00:00.000000000Z");
    let config_file = Paths::new(session.home.path()).config_file();

    let listed = serve_state::send(&session, "GET", "/api/v1/learning/proposals", None).await;
    assert_eq!(listed.status, 200, "body: {}", listed.text);
    let proposals = listed.json["data"].as_array().expect("an array");
    assert_eq!(proposals.len(), 1);
    assert_eq!(proposals[0]["id"], "prop1");
    let knob = &proposals[0]["knobs"][0];
    assert_eq!(knob["name"], "rrf_k");
    assert_eq!(knob["from"].as_f64(), Some(60.0));
    assert_eq!(knob["to"].as_f64(), Some(25.0));
    assert_ne!(knob["from"], knob["to"], "AC-13: from != to on a knob");

    let unconfirmed = serve_state::send(
        &session,
        "POST",
        "/api/v1/learning/proposals/prop1/apply",
        Some(json!({})),
    )
    .await;
    assert_eq!(unconfirmed.status, 400, "body: {}", unconfirmed.text);
    assert_eq!(unconfirmed.json["error"]["code"], "confirmation_required");
    assert!(!config_file.exists(), "a refused apply writes nothing");

    let applied = serve_state::send(
        &session,
        "POST",
        "/api/v1/learning/proposals/prop1/apply",
        Some(json!({ "confirm": true })),
    )
    .await;
    assert_eq!(applied.status, 200, "body: {}", applied.text);
    assert_eq!(applied.json["data"]["applied"], true);

    let written = std::fs::read_to_string(&config_file).expect("config.toml was written");
    assert!(written.contains("rrf_k = 25.0"), "got:\n{written}");

    let after = serve_state::send(&session, "GET", "/api/v1/learning/proposals", None).await;
    assert_eq!(
        after.json["data"].as_array().expect("an array").len(),
        0,
        "an applied proposal disappears"
    );

    // The reload is observable: the live knobs now echo the applied value.
    let knobs = serve_state::send(&session, "GET", "/api/v1/config/retrieval", None).await;
    assert_eq!(
        knobs.json["data"]["rrf_k"].as_f64(),
        Some(25.0),
        "apply must reload AppState.cfg"
    );
}

#[tokio::test]
async fn discard_retires_a_proposal_without_touching_config_toml_ac13() {
    let session = serve_state::session(false);
    seed_proposal(&conn(&session), "prop2", "2026-09-01T10:00:00.000000000Z");
    let config_file = Paths::new(session.home.path()).config_file();

    let resp = serve_state::send(
        &session,
        "POST",
        "/api/v1/learning/proposals/prop2/discard",
        None,
    )
    .await;
    assert_eq!(resp.status, 200, "body: {}", resp.text);
    assert_eq!(resp.json["data"]["discarded"], true);
    assert!(
        !config_file.exists(),
        "discard must not create or rewrite config.toml"
    );

    let after = serve_state::send(&session, "GET", "/api/v1/learning/proposals", None).await;
    assert_eq!(after.json["data"].as_array().expect("an array").len(), 0);

    let unknown = serve_state::send(
        &session,
        "POST",
        "/api/v1/learning/proposals/nosuchrun/discard",
        None,
    )
    .await;
    assert_eq!(unknown.status, 404, "body: {}", unknown.text);
    assert_eq!(unknown.json["error"]["code"], "not_found");
}

#[tokio::test]
async fn a_read_only_server_refuses_apply_and_discard_before_the_confirm_gate() {
    let session = serve_state::session(true);
    seed_proposal(&conn(&session), "prop3", "2026-09-01T10:00:00.000000000Z");

    let apply = serve_state::send(
        &session,
        "POST",
        "/api/v1/learning/proposals/prop3/apply",
        Some(json!({})),
    )
    .await;
    assert_eq!(apply.status, 405, "body: {}", apply.text);
    assert_eq!(
        apply.json["error"]["code"], "read_only",
        "read-only outranks the missing confirm (AC-19)"
    );

    let discard = serve_state::send(
        &session,
        "POST",
        "/api/v1/learning/proposals/prop3/discard",
        None,
    )
    .await;
    assert_eq!(discard.status, 405, "body: {}", discard.text);
    assert_eq!(discard.json["error"]["code"], "read_only");

    // The reads stay available on a read-only server.
    let listed = serve_state::send(&session, "GET", "/api/v1/learning/proposals", None).await;
    assert_eq!(listed.status, 200, "body: {}", listed.text);
    assert_eq!(listed.json["data"].as_array().expect("an array").len(), 1);
}

#[test]
fn post_learning_evals_is_the_eval_job_and_honors_golden_set_and_knobs() {
    let home = TempDir::new().expect("home");
    let id = support::save(&home, "postgres advisory lock migration ordering", &[]);
    let golden = home.path().join("golden.yaml");
    std::fs::write(
        &golden,
        format!("- query: postgres advisory lock migration ordering\n  relevant: [{id}]\n"),
    )
    .expect("write golden");
    let allowed = home.path().to_str().expect("utf8 path").to_string();
    let (base, token, _guard) = support::spawn_serve(&home, &["--allow-path", &allowed]);
    let client = reqwest::blocking::Client::new();

    let res = support::post(
        &client,
        &base,
        &token,
        "learning/evals",
        &json!({
            // `golden_set` is the console draft's spelling of `golden`.
            "golden_set": golden.to_str().expect("utf8 path"),
            "golden_only": true,
            "k": 1,
            "knobs": {
                "rrf_k": 42.0,
                "decay": 0.5,
                "mmr_lambda": 0.7,
                "bm25_weights": [1.0, 3.0],
                "graph_hops": 1,
                "graph_seeds": 4
            }
        }),
    );
    assert_eq!(res.status().as_u16(), 202, "the alias creates an eval job");
    let body: Value = res.json().expect("json");
    let job_id = body["data"]["job_id"].as_str().expect("job_id").to_string();
    let job = support::poll_job_terminal(&client, &base, &token, &job_id);
    assert_eq!(job["status"], "done", "job body: {job}");
    assert_eq!(job["result"]["queries"].as_u64(), Some(1));

    let listed = client
        .get(format!("{base}/api/v1/learning/evals"))
        .header("X-Comemory-Token", &token)
        .send()
        .expect("list evals");
    let rows: Value = listed.json().expect("json");
    let row = &rows["data"][0];
    assert_eq!(row["kind"], "eval");
    assert_eq!(
        row["knobs"]["rrf_k"].as_f64(),
        Some(42.0),
        "the recorded knobs are the override, not the live config: {row}"
    );
    assert_eq!(row["knobs"]["graph_hops"].as_u64(), Some(1));
    assert_eq!(row["is_baseline"], true);
}
