#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `GET|POST /api/v1/search`, `GET /api/v1/search/suggest`, and
//! `POST /api/v1/search/{query_id}/feedback` driven through the real router
//! (`tests/common/serve_state.rs`) — console-api spec AC-4 and AC-5.
//!
//! Real data throughout: memories saved through `api::save`, a real git repo
//! indexed through `api::index_code`, and feedback counters read back from
//! the store with a second connection rather than from the response body.

use comemory::api::{Ctx, index_code};
use comemory::config::{Config, Paths};
use comemory::memory::Kind;
use comemory::store::connection;
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::test_common::git_sample;
use crate::test_common::serve_state::{self, Session};

/// A second, short-lived connection onto the session's database — the
/// router's own is private.
fn conn(session: &Session) -> rusqlite::Connection {
    connection::open(Paths::new(session.home.path()).db_path()).expect("open db")
}

/// Index a real one-file git repo into the session's store through the real
/// `index-code` core. Returns the repo's tempdir, which the caller must keep
/// alive for the duration of the test.
fn index_sample_repo(session: &Session) -> TempDir {
    let repo_home = TempDir::new().expect("tempdir");
    let root = git_sample::build_sample_repo(repo_home.path());
    let paths = Paths::new(session.home.path());
    let cfg = Config::defaults();
    let mut db = connection::open(paths.db_path()).expect("open db");
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut db);
    index_code::run(
        &mut ctx,
        index_code::Request {
            repo: "sample-repo".into(),
            path: root.to_string_lossy().into_owned(),
            mode: index_code::IndexMode::Incremental,
        },
    )
    .expect("index the sample repo");
    repo_home
}

fn hits(resp: &Value) -> &Vec<Value> {
    resp["data"]["hits"]
        .as_array()
        .unwrap_or_else(|| panic!("no hits array in {resp}"))
}

fn used_count(db: &rusqlite::Connection, memory_id: &str) -> i64 {
    db.query_row(
        "SELECT used_count FROM feedback WHERE memory_id = ?1",
        [memory_id],
        |r| r.get(0),
    )
    .unwrap_or_else(|e| panic!("no feedback row for {memory_id}: {e}"))
}

#[tokio::test]
async fn a_cross_domain_search_returns_both_types_with_a_partitioned_explain_strip() {
    let session = serve_state::session(false);
    serve_state::save(
        &session,
        "the helper function is the documented fallback",
        Kind::Note,
        "sample-repo",
    );
    let _repo = index_sample_repo(&session);

    let resp = serve_state::send(
        &session,
        "POST",
        "/api/v1/search",
        Some(json!({ "q": "helper", "scope": "all" })),
    )
    .await;

    assert_eq!(resp.status, 200, "body: {}", resp.text);
    assert_eq!(resp.json["meta"]["command"], "search.console");
    assert_eq!(resp.json["data"]["fusion"]["method"], "rrf");
    assert_eq!(resp.json["data"]["tier_count"], 4);
    assert!(
        resp.json["data"]["fusion"]["k"].as_f64().unwrap() > 0.0,
        "the RRF constant the run actually used is echoed"
    );

    let hits = hits(&resp.json);
    let types: Vec<&str> = hits.iter().filter_map(|h| h["type"].as_str()).collect();
    assert!(
        types.contains(&"memory") && types.contains(&"code"),
        "scope=all must answer from both domains, got {types:?}"
    );

    for hit in hits {
        assert_eq!(hit["type"], hit["domain"], "`type` aliases `domain`");
        let parts = hit["score_parts"].as_array().expect("explain strip");
        assert!(!parts.is_empty(), "every hit explains itself");
        // Leg signals carry share 0, so the whole-strip sum IS the prior
        // partition. (A document hit would sum to 0 — it has no rerank
        // stage — but this corpus has no documents.)
        let sum: f64 = parts.iter().map(|p| p["share"].as_f64().unwrap()).sum();
        assert!(
            (sum - 1.0).abs() < 1e-6,
            "shares must partition the rerank, got {sum} for {hit}"
        );
    }

    let tier = resp.json["data"]["tier"]
        .as_u64()
        .expect("a memory hit ran the ladder");
    assert!((1..=4).contains(&tier), "tier {tier} outside the ladder");
}

#[tokio::test]
async fn explain_false_omits_the_strip_entirely() {
    let session = serve_state::session(false);
    serve_state::save(&session, "explain strip toggle", Kind::Note, "app");

    let resp = serve_state::send(
        &session,
        "POST",
        "/api/v1/search",
        Some(json!({ "q": "explain", "explain": false })),
    )
    .await;

    assert_eq!(resp.status, 200, "body: {}", resp.text);
    let hits = hits(&resp.json);
    assert!(!hits.is_empty());
    for hit in hits {
        assert!(
            hit.as_object().unwrap().get("score_parts").is_none(),
            "explain:false must omit the key, not null it: {hit}"
        );
    }
}

#[tokio::test]
async fn two_kinds_are_rejected_and_one_kind_narrows() {
    let session = serve_state::session(false);
    serve_state::save(&session, "a decision about caching", Kind::Decision, "app");
    serve_state::save(&session, "a bug about caching", Kind::Bug, "app");

    let two = serve_state::send(
        &session,
        "POST",
        "/api/v1/search",
        Some(json!({ "q": "caching", "kinds": ["bug", "decision"] })),
    )
    .await;
    assert_eq!(two.status, 400, "body: {}", two.text);
    assert_eq!(two.json["ok"], false);
    assert_eq!(two.json["error"]["code"], "bad_request");
    assert!(
        two.json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("one kind per query"),
        "message: {}",
        two.json["error"]["message"]
    );

    let one = serve_state::send(
        &session,
        "POST",
        "/api/v1/search",
        Some(json!({ "q": "caching", "kinds": ["bug"] })),
    )
    .await;
    assert_eq!(one.status, 200, "body: {}", one.text);
    let ids: Vec<&str> = hits(&one.json)
        .iter()
        .filter_map(|h| h["id"].as_str())
        .collect();
    assert_eq!(ids.len(), 1, "only the bug is in scope, got {ids:?}");
}

#[tokio::test]
async fn scope_maps_onto_the_pipelines_domains_and_an_unknown_one_is_rejected() {
    let session = serve_state::session(false);
    serve_state::save(&session, "the helper memory", Kind::Note, "sample-repo");
    let _repo = index_sample_repo(&session);

    let memories = serve_state::send(
        &session,
        "GET",
        "/api/v1/search?q=helper&scope=memories",
        None,
    )
    .await;
    assert_eq!(memories.status, 200, "body: {}", memories.text);
    assert!(
        hits(&memories.json).iter().all(|h| h["type"] == "memory"),
        "scope=memories is memory-only"
    );

    let code = serve_state::send(&session, "GET", "/api/v1/search?q=helper&scope=code", None).await;
    assert_eq!(code.status, 200, "body: {}", code.text);
    assert!(!hits(&code.json).is_empty());
    assert!(hits(&code.json).iter().all(|h| h["type"] == "code"));
    assert!(
        code.json["data"]["tier"].is_null(),
        "a code-only page ran no memory ladder, so it reports no tier"
    );

    let bad = serve_state::send(&session, "GET", "/api/v1/search?q=helper&scope=nope", None).await;
    assert_eq!(bad.status, 400, "body: {}", bad.text);
    assert_eq!(bad.json["error"]["code"], "bad_request");
}

#[tokio::test]
async fn per_hit_feedback_records_the_same_counters_as_the_list_form() {
    let session = serve_state::session(false);
    // Deliberately dissimilar bodies beyond the shared query term: the
    // diversify stage collapses near-duplicates by SimHash, and two
    // one-word-apart memories would arrive as a single hit.
    serve_state::save(
        &session,
        "caching invalidation happens on every write to the edge tier",
        Kind::Note,
        "app",
    );
    serve_state::save(
        &session,
        "caching saves a second round trip to postgres for hot rows",
        Kind::Note,
        "app",
    );

    let search = serve_state::send(
        &session,
        "POST",
        "/api/v1/search",
        Some(json!({ "q": "caching", "scope": "memories" })),
    )
    .await;
    assert_eq!(search.status, 200, "body: {}", search.text);
    let query_id = search.json["data"]["query_id"]
        .as_str()
        .expect("a tracked search logs a query id")
        .to_string();
    let ids: Vec<String> = hits(&search.json)
        .iter()
        .filter_map(|h| h["id"].as_str().map(str::to_string))
        .collect();
    assert_eq!(ids.len(), 2, "both seeded memories matched");

    let per_hit = serve_state::send(
        &session,
        "POST",
        &format!("/api/v1/search/{query_id}/feedback"),
        Some(json!({ "hit_id": ids[0], "signal": "used", "source": "explicit" })),
    )
    .await;
    assert_eq!(per_hit.status, 200, "body: {}", per_hit.text);
    assert_eq!(per_hit.json["meta"]["command"], "search.feedback");
    assert_eq!(per_hit.json["data"]["used"], 1);

    let list_form = serve_state::send(
        &session,
        "POST",
        "/api/v1/feedback",
        Some(json!({ "query_id": query_id, "used": [ids[1]] })),
    )
    .await;
    assert_eq!(list_form.status, 200, "body: {}", list_form.text);

    let db = conn(&session);
    assert_eq!(
        used_count(&db, &ids[0]),
        used_count(&db, &ids[1]),
        "the per-hit route and the list route write the same counter"
    );
    assert_eq!(used_count(&db, &ids[0]), 1);
}

#[tokio::test]
async fn ignored_records_an_irrelevant_verdict_and_an_unknown_signal_is_rejected() {
    let session = serve_state::session(false);
    serve_state::save(&session, "retention policy notes", Kind::Note, "app");

    let search = serve_state::send(
        &session,
        "POST",
        "/api/v1/search",
        Some(json!({ "q": "retention", "scope": "memories" })),
    )
    .await;
    let query_id = search.json["data"]["query_id"]
        .as_str()
        .expect("query id")
        .to_string();
    let hit_id = hits(&search.json)[0]["id"].as_str().unwrap().to_string();

    let ignored = serve_state::send(
        &session,
        "POST",
        &format!("/api/v1/search/{query_id}/feedback"),
        Some(json!({ "hit_id": hit_id, "signal": "ignored" })),
    )
    .await;
    assert_eq!(ignored.status, 200, "body: {}", ignored.text);
    assert_eq!(ignored.json["data"]["irrelevant"], 1);
    assert_eq!(ignored.json["data"]["used"], 0);

    let irrelevant: i64 = conn(&session)
        .query_row(
            "SELECT irrelevant_count FROM feedback WHERE memory_id = ?1",
            [&hit_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(irrelevant, 1);

    let bogus = serve_state::send(
        &session,
        "POST",
        &format!("/api/v1/search/{query_id}/feedback"),
        Some(json!({ "hit_id": hit_id, "signal": "maybe" })),
    )
    .await;
    assert_eq!(bogus.status, 400, "body: {}", bogus.text);
    assert_eq!(bogus.json["error"]["code"], "bad_request");
}

#[tokio::test]
async fn feedback_is_refused_on_a_read_only_server_but_search_still_answers() {
    let session = serve_state::session(true);

    let search = serve_state::send(&session, "GET", "/api/v1/search?q=anything", None).await;
    assert_eq!(search.status, 200, "body: {}", search.text);
    assert!(
        search.json["data"]["query_id"].is_null(),
        "a read-only server records no retrieval_log row"
    );

    let feedback = serve_state::send(
        &session,
        "POST",
        "/api/v1/search/q-20260901-aabbccdd/feedback",
        Some(json!({ "hit_id": "aaaa1111", "signal": "used" })),
    )
    .await;
    assert_eq!(feedback.status, 405, "body: {}", feedback.text);
    assert_eq!(feedback.json["error"]["code"], "read_only");
}

#[tokio::test]
async fn suggest_returns_mined_expansions_and_the_real_query_log() {
    let session = serve_state::session(false);
    serve_state::save(&session, "frontmatter is the contract", Kind::Note, "app");
    {
        let db = conn(&session);
        db.execute(
            "INSERT INTO query_expansions(term, expansion, support, last_mined) \
             VALUES ('frontmatter', 'yaml', 4, '2026-08-01T00:00:00Z')",
            [],
        )
        .unwrap();
    }

    let search = serve_state::send(
        &session,
        "POST",
        "/api/v1/search",
        Some(json!({ "q": "frontmatter contract", "scope": "memories" })),
    )
    .await;
    assert_eq!(search.status, 200, "body: {}", search.text);

    let resp = serve_state::send(
        &session,
        "GET",
        "/api/v1/search/suggest?q=frontmatter",
        None,
    )
    .await;

    assert_eq!(resp.status, 200, "body: {}", resp.text);
    assert_eq!(resp.json["meta"]["command"], "search.suggest");
    let expansions = resp.json["data"]["expansions"].as_array().unwrap();
    assert_eq!(expansions.len(), 1);
    assert_eq!(expansions[0]["term"], "frontmatter");
    assert_eq!(expansions[0]["expansion"], "yaml");
    assert_eq!(expansions[0]["support"], 4);

    let recent = resp.json["data"]["recent"].as_array().unwrap();
    assert_eq!(
        recent.len(),
        1,
        "the one real tracked search is the whole log: {recent:?}"
    );
    assert_eq!(recent[0]["query"], "frontmatter contract");
    assert_eq!(recent[0]["query_id"], search.json["data"]["query_id"]);

    let empty = serve_state::send(&session, "GET", "/api/v1/search/suggest?q=", None).await;
    assert_eq!(empty.status, 400, "body: {}", empty.text);
    assert_eq!(empty.json["error"]["code"], "bad_request");
}
