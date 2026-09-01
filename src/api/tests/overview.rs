#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `api::overview::run` against a real store (console-api spec AC-3):
//! memories saved through `api::save`, a real git repo indexed through
//! `api::index_code`, eval history written through the production
//! `store::eval_runs` writer. Every counter is compared against the same
//! thing counted a second way rather than against a hardcoded number.

use comemory::api::{Ctx, index_code, overview, save};
use comemory::config::{Config, Paths};
use comemory::memory::Kind;
use comemory::store::{connection, eval_runs, index_runs};
use tempfile::TempDir;

use crate::test_common::git_sample;

fn fresh_paths(dir: &TempDir) -> Paths {
    let paths = Paths::new(dir.path().to_path_buf());
    paths.ensure_dirs().unwrap();
    paths
}

/// Save one memory through the real command core.
fn seed_memory(paths: &Paths, conn: &mut rusqlite::Connection, body: &str, repo: &str) -> String {
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(paths, &cfg, conn);
    save::run(
        &mut ctx,
        save::Request {
            body: body.to_string(),
            title: None,
            kind: Kind::Note,
            repo: repo.to_string(),
            tags: Vec::new(),
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

fn run(paths: &Paths, conn: &mut rusqlite::Connection, repo: Option<&str>) -> overview::Response {
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(paths, &cfg, conn);
    overview::run(
        &mut ctx,
        overview::Request {
            repo: repo.map(str::to_string),
        },
    )
    .unwrap()
}

fn seed_eval_run(conn: &rusqlite::Connection, id: &str, at: &str, recall: f64) {
    eval_runs::insert(
        conn,
        &eval_runs::NewRun {
            id,
            kind: "eval",
            at,
            golden_pairs: 12,
            k: 5,
            recall,
            mrr: recall / 2.0,
            knobs: "{}",
            applied: false,
        },
    )
    .unwrap();
}

#[test]
fn a_data_dir_without_a_database_reports_an_empty_overview_and_creates_nothing() {
    let dir = TempDir::new().unwrap();
    let paths = fresh_paths(&dir);
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);

    let resp = overview::run(&mut ctx, overview::Request::default()).unwrap();

    assert_eq!(resp.counters.memories, 0);
    assert_eq!(resp.counters.graph_edges, 0);
    assert_eq!(resp.index_state.status, "unknown");
    assert!(resp.index_state.repos.is_empty());
    assert!(resp.last_run.is_none());
    assert!(resp.metrics.is_none());
    assert!(resp.eval_series.is_empty());
    assert!(resp.recent_memories.is_empty());
    assert!(
        !paths.db_path().exists(),
        "asking for an overview must not create and migrate a database"
    );
}

#[test]
fn counters_match_the_live_corpus_and_recent_memories_are_capped_at_four() {
    let dir = TempDir::new().unwrap();
    let paths = fresh_paths(&dir);
    let mut conn = connection::open(paths.db_path()).unwrap();
    for i in 0..6 {
        seed_memory(
            &paths,
            &mut conn,
            &format!("overview seed number {i}"),
            "app",
        );
    }

    let resp = run(&paths, &mut conn, None);

    let live: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE deleted_at IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let edges: i64 = conn
        .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))
        .unwrap();
    assert_eq!(resp.counters.memories as i64, live);
    assert_eq!(resp.counters.memories, 6);
    assert_eq!(resp.counters.trashed, 0);
    assert_eq!(resp.counters.graph_edges as i64, edges);
    assert!(resp.counters.db_bytes > 0);

    assert_eq!(
        resp.recent_memories.len(),
        4,
        "the Overview list shows at most four"
    );
    let created: Vec<&str> = resp
        .recent_memories
        .iter()
        .map(|m| m.created.as_str())
        .collect();
    let mut sorted = created.clone();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    assert_eq!(created, sorted, "recent memories are newest first");
}

#[test]
fn the_repo_filter_narrows_the_counters_and_the_recent_list() {
    let dir = TempDir::new().unwrap();
    let paths = fresh_paths(&dir);
    let mut conn = connection::open(paths.db_path()).unwrap();
    seed_memory(&paths, &mut conn, "one filed under alpha", "alpha");
    seed_memory(&paths, &mut conn, "two filed under alpha", "alpha");
    seed_memory(&paths, &mut conn, "three filed under beta", "beta");

    let all = run(&paths, &mut conn, None);
    let scoped = run(&paths, &mut conn, Some("alpha"));

    assert_eq!(all.counters.memories, 3);
    assert_eq!(scoped.counters.memories, 2);
    assert_eq!(scoped.recent_memories.len(), 2);
    assert!(scoped.recent_memories.iter().all(|m| m.repo == "alpha"));
    assert_eq!(
        scoped.counters.db_bytes, all.counters.db_bytes,
        "a database has one size no matter which repo asks"
    );
}

#[test]
fn last_run_is_absent_until_a_real_index_run_and_then_mirrors_the_newest_row() {
    let dir = TempDir::new().unwrap();
    let paths = fresh_paths(&dir);
    let mut conn = connection::open(paths.db_path()).unwrap();

    let before = run(&paths, &mut conn, None);
    assert!(before.last_run.is_none(), "no index run has happened yet");
    assert_eq!(before.index_state.status, "unknown");

    let repo_root = git_sample::build_sample_repo(dir.path());
    {
        let cfg = Config::defaults();
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        index_code::run(
            &mut ctx,
            index_code::Request {
                repo: "sample-repo".into(),
                path: repo_root.to_string_lossy().into_owned(),
                mode: index_code::IndexMode::Incremental,
            },
        )
        .unwrap();
    }

    let after = run(&paths, &mut conn, None);
    let newest = index_runs::newest(&conn)
        .unwrap()
        .expect("one run recorded");
    let last = after.last_run.expect("overview reports the run");

    assert_eq!(last.id, newest.id);
    assert_eq!(last.repo, "sample-repo");
    assert_eq!(last.mode, "incremental");
    assert_eq!(last.outcome, "ok");
    assert_eq!(last.started_at, newest.started_at);
    assert_eq!(last.finished_at, newest.finished_at);
    assert_eq!(last.files_indexed, newest.files_indexed);
    assert_eq!(last.symbols, newest.symbols);
    assert!(last.files_indexed > 0, "the sample repo has one .rs file");
    assert!(last.error.is_none());

    let cochange: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM edges WHERE rel = 'co_changed'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let imports: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM edges WHERE rel = 'imports'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(last.edges.cochange as i64, cochange);
    assert_eq!(last.edges.imports as i64, imports);

    assert_eq!(after.counters.code_symbols, newest.symbols);
    assert_eq!(after.index_state.repos.len(), 1);
    assert_eq!(after.index_state.repos[0].repo, "sample-repo");
    assert_eq!(
        after.index_state.status, after.index_state.repos[0].status,
        "one repo means the rolled-up status IS that repo's status"
    );
    assert!(
        !after.index_state.checked_at.is_empty(),
        "the freshness probe stamps when it ran"
    );
}

#[test]
fn metrics_come_from_the_newest_eval_run_and_the_series_reads_oldest_first() {
    let dir = TempDir::new().unwrap();
    let paths = fresh_paths(&dir);
    let mut conn = connection::open(paths.db_path()).unwrap();
    seed_eval_run(&conn, "1111111111111111", "2026-08-01T00:00:00Z", 0.40);
    seed_eval_run(&conn, "2222222222222222", "2026-08-02T00:00:00Z", 0.55);
    seed_eval_run(&conn, "3333333333333333", "2026-08-03T00:00:00Z", 0.61);

    let resp = run(&paths, &mut conn, None);

    let metrics = resp.metrics.expect("an eval run exists");
    assert_eq!(metrics.at, "2026-08-03T00:00:00Z");
    assert_eq!(metrics.recall_at_k, 0.61);
    assert_eq!(metrics.mrr, 0.305);
    assert_eq!(metrics.k, 5);
    assert_eq!(metrics.golden_queries, 12);

    let ats: Vec<&str> = resp.eval_series.iter().map(|p| p.at.as_str()).collect();
    assert_eq!(
        ats,
        vec![
            "2026-08-01T00:00:00Z",
            "2026-08-02T00:00:00Z",
            "2026-08-03T00:00:00Z"
        ],
        "a sparkline reads left to right, so the series is oldest first"
    );
    assert_eq!(resp.eval_series[0].recall_at_k, 0.40);
    assert_eq!(resp.eval_series[0].kind, "eval");
}

#[test]
fn eval_series_honors_its_limit_and_stays_oldest_first() {
    let dir = TempDir::new().unwrap();
    let paths = fresh_paths(&dir);
    let mut conn = connection::open(paths.db_path()).unwrap();
    for day in 1..=5 {
        seed_eval_run(
            &conn,
            &format!("{day:016x}"),
            &format!("2026-08-0{day}T00:00:00Z"),
            f64::from(day) / 10.0,
        );
    }

    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let series = overview::eval_series(&mut ctx, 2).unwrap();

    let ats: Vec<&str> = series.iter().map(|p| p.at.as_str()).collect();
    assert_eq!(
        ats,
        vec!["2026-08-04T00:00:00Z", "2026-08-05T00:00:00Z"],
        "the two NEWEST runs, presented oldest first"
    );
}

#[test]
fn eval_series_on_a_missing_database_is_empty_and_creates_nothing() {
    let dir = TempDir::new().unwrap();
    let paths = fresh_paths(&dir);
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);

    assert!(overview::eval_series(&mut ctx, 20).unwrap().is_empty());
    assert!(!paths.db_path().exists());
}
