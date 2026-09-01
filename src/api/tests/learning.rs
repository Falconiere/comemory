#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Mirror test for `src/api/learning.rs`: the four console reads against a
//! real migrated `comemory.db` — real `eval_runs` rows, real mined
//! `query_expansions` rows, and a golden harvest grown from a real
//! save → search → feedback round trip (no fixture doubles).

use comemory::api::{self, Ctx};
use comemory::config::{Config, Paths};
use comemory::memory::Kind;
use comemory::store::{connection, eval_runs};
use tempfile::TempDir;

/// A data-dir under a fresh temp home. The database is NOT created — the
/// missing-db tests depend on that, and every other test opens it itself.
fn paths_in(home: &TempDir) -> Paths {
    Paths::new(home.path().join(".comemory"))
}

/// Insert one `eval_runs` row with a known kind/timestamp/recall.
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
            mrr: recall / 2.0,
            knobs: "{\"rrf_k\":60.0}",
            applied: false,
        },
    )
    .expect("insert eval_runs row");
}

#[test]
fn every_read_answers_empty_on_a_data_dir_with_no_database() {
    let home = TempDir::new().expect("tempdir");
    let paths = paths_in(&home);
    let cfg = Config::defaults();
    let mut ctx = Ctx::lazy(&paths, &cfg);

    let summary = api::learning::summary(&mut ctx).expect("summary");
    assert_eq!(summary.feedback_events, 0);
    assert_eq!(summary.implicit_share, 0.0);
    assert_eq!(summary.expansions, 0);
    assert!(summary.latest.is_none());
    assert!(summary.best_delta.is_none());

    assert!(
        api::learning::evals(&mut ctx, 20)
            .expect("evals")
            .is_empty()
    );
    let page = api::learning::expansions(&mut ctx, 20, 0).expect("expansions");
    assert!(page.items.is_empty());
    assert_eq!(page.total, Some(0));
    let golden = api::learning::golden_set(&mut ctx, None).expect("golden set");
    assert_eq!(golden.count, 0);

    assert!(
        !paths.db_path().exists(),
        "a learning READ must never create comemory.db"
    );
}

#[test]
fn evals_derives_delta_is_baseline_and_is_best_from_real_rows() {
    let home = TempDir::new().expect("tempdir");
    let paths = paths_in(&home);
    paths.ensure_dirs().expect("dirs");
    let mut conn = connection::open(paths.db_path()).expect("open db");
    seed_run(
        &conn,
        "run1",
        "eval",
        "2026-08-30T10:00:00.000000000Z",
        0.40,
    );
    seed_run(
        &conn,
        "run2",
        "tune",
        "2026-08-31T10:00:00.000000000Z",
        0.60,
    );
    seed_run(
        &conn,
        "run3",
        "eval",
        "2026-09-01T10:00:00.000000000Z",
        0.50,
    );

    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let rows = api::learning::evals(&mut ctx, 20).expect("evals");

    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].run.id, "run3", "newest first");
    // Each row's delta is measured against the chronologically previous one.
    assert_eq!(rows[0].delta, Some(0.50 - 0.60));
    assert_eq!(rows[1].delta, Some(0.60 - 0.40));
    assert_eq!(rows[2].delta, None, "the oldest row has nothing before it");

    assert!(rows[0].is_baseline, "run3 is a plain eval");
    assert!(!rows[1].is_baseline, "run2 is a tune");
    assert!(rows[2].is_baseline);

    assert!(rows[1].is_best, "0.60 is the highest recall in the page");
    assert!(!rows[0].is_best);
    assert!(!rows[2].is_best);
}

#[test]
fn summary_reports_the_newest_run_and_the_best_gain_over_its_baseline() {
    let home = TempDir::new().expect("tempdir");
    let paths = paths_in(&home);
    paths.ensure_dirs().expect("dirs");
    let mut conn = connection::open(paths.db_path()).expect("open db");
    seed_run(
        &conn,
        "run1",
        "eval",
        "2026-08-30T10:00:00.000000000Z",
        0.40,
    );
    seed_run(
        &conn,
        "run2",
        "tune",
        "2026-08-31T10:00:00.000000000Z",
        0.60,
    );
    seed_run(
        &conn,
        "run3",
        "eval",
        "2026-09-01T10:00:00.000000000Z",
        0.50,
    );

    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let summary = api::learning::summary(&mut ctx).expect("summary");

    let latest = summary.latest.expect("a latest run");
    assert_eq!(latest.id, "run3");
    assert_eq!(latest.kind, "eval");
    assert_eq!(latest.recall_at_k, 0.50);
    assert_eq!(latest.k, 3);
    assert_eq!(latest.golden_pairs, 12);
    // run2 (tune, 0.60) against run1 (the nearest EARLIER eval, 0.40).
    assert_eq!(summary.best_delta, Some(0.60 - 0.40));
}

#[test]
fn summary_counts_feedback_and_its_implicit_share_from_real_verdicts() {
    let home = TempDir::new().expect("tempdir");
    let paths = paths_in(&home);
    paths.ensure_dirs().expect("dirs");
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let id = save_note(&mut ctx, "sqlite wal checkpoint starvation under load");
    let query_id = search_query_id(&mut ctx, "sqlite wal checkpoint starvation under load");
    api::feedback::run(
        &mut ctx,
        api::feedback::Request {
            query_id,
            used: vec![id],
            irrelevant: Vec::new(),
            used_code: Vec::new(),
            irrelevant_code: Vec::new(),
        },
    )
    .expect("record feedback");

    let summary = api::learning::summary(&mut ctx).expect("summary");
    assert_eq!(summary.feedback_events, 1);
    assert_eq!(summary.used, 1);
    assert_eq!(summary.irrelevant, 0);
    assert_eq!(
        summary.implicit_share, 0.0,
        "a `comemory feedback` verdict is provenance='manual'"
    );
}

#[test]
fn golden_set_merges_the_real_harvest_with_a_file() {
    let home = TempDir::new().expect("tempdir");
    let paths = paths_in(&home);
    paths.ensure_dirs().expect("dirs");
    let mut conn = connection::open(paths.db_path()).expect("open db");
    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let query = "postgres advisory lock ordering across migrations";
    let id = save_note(&mut ctx, query);
    let query_id = search_query_id(&mut ctx, query);
    api::feedback::run(
        &mut ctx,
        api::feedback::Request {
            query_id,
            used: vec![id.clone()],
            irrelevant: Vec::new(),
            used_code: Vec::new(),
            irrelevant_code: Vec::new(),
        },
    )
    .expect("record feedback");

    let harvest_only = api::learning::golden_set(&mut ctx, None).expect("golden set");
    assert_eq!(harvest_only.harvested, 1, "one used verdict, one pair");
    assert_eq!(harvest_only.from_file, 0);
    assert_eq!(harvest_only.count, 1);
    assert_eq!(harvest_only.pairs[0].query, query);
    assert_eq!(harvest_only.pairs[0].relevant, vec![id]);

    let file = home.path().join("golden.yaml");
    std::fs::write(
        &file,
        "- query: an unrelated hand-written query\n  relevant: [aaaaaaaa]\n",
    )
    .expect("write golden file");
    let merged = api::learning::golden_set(&mut ctx, file.to_str()).expect("golden set");
    assert_eq!(merged.harvested, 1);
    assert_eq!(merged.from_file, 1);
    assert_eq!(merged.count, 2, "different keys coexist after the merge");
}

#[test]
fn expansions_pages_mined_rows_strongest_support_first() {
    let home = TempDir::new().expect("tempdir");
    let paths = paths_in(&home);
    paths.ensure_dirs().expect("dirs");
    let mut conn = connection::open(paths.db_path()).expect("open db");
    for (term, expansion, support) in [
        ("lock", "advisory", 5_i64),
        ("wal", "checkpoint", 9),
        ("index", "btree", 1),
    ] {
        conn.execute(
            "INSERT INTO query_expansions(term, expansion, support, last_mined) \
             VALUES (?1, ?2, ?3, '2026-09-01T00:00:00.000000000Z')",
            rusqlite::params![term, expansion, support],
        )
        .expect("insert expansion");
    }

    let cfg = Config::defaults();
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    let first = api::learning::expansions(&mut ctx, 2, 0).expect("expansions");
    assert_eq!(first.total, Some(3));
    assert!(first.has_more);
    assert_eq!(first.items.len(), 2);
    assert_eq!(first.items[0].from, "wal");
    assert_eq!(first.items[0].to, "checkpoint");
    assert_eq!(first.items[0].count, 9);
    assert_eq!(first.items[1].from, "lock");

    let second = api::learning::expansions(&mut ctx, 2, 2).expect("expansions page 2");
    assert_eq!(second.items.len(), 1);
    assert_eq!(second.items[0].from, "index");
    assert!(!second.has_more);

    let all = api::learning::expansions(&mut ctx, 0, 0).expect("expansions, limit 0");
    assert_eq!(all.items.len(), 3, "limit 0 is the `all` sentinel");
    assert!(!all.has_more);
}

/// Save one note through the real command core and return its id.
fn save_note(ctx: &mut Ctx<'_>, body: &str) -> String {
    let req = api::save::Request {
        body: body.to_string(),
        title: None,
        kind: Kind::Note,
        repo: "comemory".to_string(),
        tags: Vec::new(),
        author: String::new(),
        quality: 3,
        supersedes: Vec::new(),
        vector: None,
        ref_file: Vec::new(),
        ref_symbol: Vec::new(),
    };
    api::save::run(ctx, req, false, None).expect("seed save").id
}

/// Run a tracked search (so a `retrieval_log` row exists) and return its
/// query id — the handle `comemory feedback` records verdicts against.
fn search_query_id(ctx: &mut Ctx<'_>, query: &str) -> String {
    let req = api::search::Request {
        query: query.to_string(),
        k: None,
        offset: 0,
        repo: None,
        kind: None,
        vector: None,
        since: None,
        until: None,
        as_of: None,
    };
    api::search::run(ctx, req, true)
        .expect("seed search")
        .query_id
        .expect("a tracked search records a query id")
}
