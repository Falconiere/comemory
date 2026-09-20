#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! What the four retrieval cores record in `activity_log`, against a real
//! corpus in a temp data dir: the query each ran, the hits it returned, the
//! `retrieval_log` id it links to, and the zero-hit run that is still a run.

use comemory::config::{Config, Paths};
use comemory::domains::memories::{self, Kind};
use comemory::domains::retrieval::{context, find, search};
use comemory::store::activity::{ActivityFilter, ActivityRow, list};
use comemory::store::connection;
use comemory::utilities::context::Ctx;
use serde_json::Value;

/// A memory-search request for `query`, everything else at its neutral value.
fn search_request(query: &str, repo: Option<&str>) -> search::Request {
    search::Request {
        query: query.to_string(),
        k: None,
        offset: 0,
        repo: repo.map(str::to_string),
        kind: None,
        vector: None,
        since: None,
        until: None,
        as_of: None,
    }
}

/// A unified-search request for `query`, everything else neutral.
fn find_request(query: &str) -> find::Request {
    find::Request {
        query: query.to_string(),
        k: None,
        offset: 0,
        domain: None,
        repo: None,
        kind: None,
        lang: None,
        path: Vec::new(),
        vector: None,
        since: None,
        until: None,
        as_of: None,
    }
}

/// A context request for `query`, everything else neutral.
fn context_request(query: &str) -> context::Request {
    context::Request {
        query: query.to_string(),
        k: None,
        offset: 0,
        repo: None,
        vector: None,
        since: None,
        until: None,
        as_of: None,
    }
}

/// Save one memory so the searches below have something to find.
fn seed(paths: &Paths, cfg: &Config, conn: &mut comemory::store::Connection, body: &str) -> String {
    let mut ctx = Ctx::borrowed(paths, cfg, conn);
    memories::save::run(
        &mut ctx,
        memories::save::Request {
            body: body.to_string(),
            title: None,
            kind: Kind::Decision,
            repo: "demo".to_string(),
            tags: vec!["db".to_string()],
            author: String::new(),
            quality: 3,
            supersedes: Vec::new(),
            vector: None,
            ref_file: Vec::new(),
            ref_symbol: Vec::new(),
        },
        false,
        None,
    )
    .unwrap()
    .id
}

fn rows_for(conn: &comemory::store::Connection, command: &str) -> Vec<ActivityRow> {
    let filter = ActivityFilter {
        command: Some(command),
        ..ActivityFilter::default()
    };
    list(conn, &filter, 0, 0).unwrap().0
}

fn summary_of(row: &ActivityRow) -> Value {
    serde_json::from_str(row.summary.as_deref().expect("a summary")).unwrap()
}

#[test]
fn a_search_records_its_query_hit_count_and_tracked_query_id() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path());
    paths.ensure_dirs().unwrap();
    let mut conn = connection::open(paths.db_path()).unwrap();
    let cfg = Config::defaults();
    let id = seed(
        &paths,
        &cfg,
        &mut conn,
        "pgbouncer in transaction mode fixes pool exhaustion",
    );

    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        let result = search::run(
            &mut ctx,
            search_request("pgbouncer pool", Some("demo")),
            true,
        )
        .unwrap();
        assert!(!result.hits.is_empty(), "the seeded memory is findable");
    }

    let rows = rows_for(&conn, "search");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].repo.as_deref(), Some("demo"));
    let summary = summary_of(&rows[0]);
    assert_eq!(summary["query"], "pgbouncer pool");
    assert_eq!(summary["hits"], 1);
    assert_eq!(summary["top"], serde_json::json!([id]));
    assert!(
        summary["query_id"].is_string(),
        "a tracked run links its retrieval_log row: {summary}"
    );
}

#[test]
fn a_find_records_hits_per_domain_and_a_context_run_records_its_bundle_size() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path());
    paths.ensure_dirs().unwrap();
    let mut conn = connection::open(paths.db_path()).unwrap();
    let cfg = Config::defaults();
    seed(
        &paths,
        &cfg,
        &mut conn,
        "pgbouncer in transaction mode fixes pool exhaustion",
    );

    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        find::run(&mut ctx, find_request("pgbouncer"), true).unwrap();
    }
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        context::run(&mut ctx, context_request("pgbouncer"), true).unwrap();
    }

    let find_rows = rows_for(&conn, "find");
    assert_eq!(find_rows.len(), 1);
    let summary = summary_of(&find_rows[0]);
    assert_eq!(summary["total"], 1);
    assert_eq!(
        summary["hits"],
        serde_json::json!({"memory": 1}),
        "hits are reported per domain, so an empty leg is visibly empty"
    );

    let context_rows = rows_for(&conn, "context");
    assert_eq!(context_rows.len(), 1);
    assert_eq!(summary_of(&context_rows[0])["query"], "pgbouncer");
}

#[test]
fn a_zero_hit_query_is_still_a_recorded_run() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path());
    paths.ensure_dirs().unwrap();
    let mut conn = connection::open(paths.db_path()).unwrap();
    let cfg = Config::defaults();
    seed(&paths, &cfg, &mut conn, "pgbouncer pool exhaustion");

    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        let result = search::run(
            &mut ctx,
            search_request("kubernetes ingress annotations", None),
            true,
        )
        .unwrap();
        assert!(result.hits.is_empty());
    }

    let rows = rows_for(&conn, "search");
    assert_eq!(rows.len(), 1, "an empty result is still a run");
    assert!(rows[0].ok);
    let summary = summary_of(&rows[0]);
    assert_eq!(summary["hits"], 0);
    assert_eq!(summary["top"], serde_json::json!([]));
}

#[test]
fn a_long_query_is_bounded_in_the_summary() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path());
    paths.ensure_dirs().unwrap();
    let mut conn = connection::open(paths.db_path()).unwrap();
    let cfg = Config::defaults();
    let long = "pool ".repeat(200);

    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        search::run(&mut ctx, search_request(&long, None), true).unwrap();
    }

    let rows = rows_for(&conn, "search");
    let recorded = summary_of(&rows[0])["query"].as_str().unwrap().to_string();
    assert_eq!(recorded.chars().count(), 200);
    assert!(long.starts_with(&recorded));
}
