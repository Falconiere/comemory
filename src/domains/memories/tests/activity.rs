#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! What the four memory cores record in `activity_log`, proven by calling
//! them against a real temp data dir and reading the rows back: one row per
//! run, the summary each carries, and the `ok = 0` row a refused save writes.

use comemory::config::{Config, Paths};
use comemory::domains::memories::{self, Kind};
use comemory::store::activity::{ActivityFilter, ActivityRow, list};
use comemory::store::connection;
use comemory::utilities::context::Ctx;
use serde_json::Value;

fn request(body: &str) -> memories::save::Request {
    memories::save::Request {
        body: body.to_string(),
        title: Some("Pool the connections".to_string()),
        kind: Kind::Decision,
        repo: "demo".to_string(),
        tags: vec!["db".to_string(), "postgres".to_string()],
        author: String::new(),
        quality: 3,
        supersedes: Vec::new(),
        vector: None,
        ref_file: Vec::new(),
        ref_symbol: Vec::new(),
    }
}

/// The rows a run wrote, newest first.
fn rows(conn: &comemory::store::Connection) -> Vec<ActivityRow> {
    list(conn, &ActivityFilter::default(), 0, 0).unwrap().0
}

fn summary_of(row: &ActivityRow) -> Value {
    serde_json::from_str(row.summary.as_deref().expect("a summary")).unwrap()
}

#[test]
fn save_delete_and_restore_each_record_one_row_with_their_own_summary() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path());
    paths.ensure_dirs().unwrap();
    let mut conn = connection::open(paths.db_path()).unwrap();
    let cfg = Config::defaults();

    let saved = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        memories::save::run(&mut ctx, request("use pgbouncer"), false, None).unwrap()
    };
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        memories::delete::run(&mut ctx, &saved.id).unwrap();
    }
    {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        memories::restore::run(&mut ctx, &saved.id).unwrap();
    }

    let recorded = rows(&conn);
    assert_eq!(
        recorded
            .iter()
            .map(|r| r.command.as_str())
            .collect::<Vec<_>>(),
        ["restore", "delete", "save"],
        "one row per run, newest first"
    );
    assert!(recorded.iter().all(|r| r.ok && r.source == "cli"));

    let save_row = recorded.iter().find(|r| r.command == "save").unwrap();
    assert_eq!(save_row.repo.as_deref(), Some("demo"));
    let summary = summary_of(save_row);
    assert_eq!(summary["id"], saved.id);
    assert_eq!(summary["title"], "Pool the connections");
    assert_eq!(summary["kind"], "decision");
    assert_eq!(summary["tags"], 2);
    assert_eq!(summary["supersedes"], 0);
    assert_eq!(summary["created"], true);

    for command in ["delete", "restore"] {
        let row = recorded.iter().find(|r| r.command == command).unwrap();
        assert_eq!(
            summary_of(row)["id"],
            saved.id,
            "{command} names the memory it touched"
        );
    }
}

#[test]
fn an_update_records_the_fields_it_changed_and_no_second_save_row() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path());
    paths.ensure_dirs().unwrap();
    let mut conn = connection::open(paths.db_path()).unwrap();
    let cfg = Config::defaults();

    let saved = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        memories::save::run(&mut ctx, request("use pgbouncer"), false, None).unwrap()
    };
    let patched = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        memories::update::run(
            &mut ctx,
            &saved.id,
            memories::update::Request {
                kind: None,
                repo: None,
                tags: None,
                quality: Some(5),
                body: None,
                title: None,
            },
        )
        .unwrap()
    };

    let recorded = rows(&conn);
    assert_eq!(
        recorded.iter().filter(|r| r.command == "save").count(),
        1,
        "the patch records one update row, not a second save row"
    );
    let update_row = recorded.iter().find(|r| r.command == "update").unwrap();
    let summary = summary_of(update_row);
    assert_eq!(summary["id"], patched.id);
    assert_eq!(summary["fields"], serde_json::json!(["quality"]));
}

#[test]
fn a_refused_save_records_a_failed_row_and_writes_no_memory() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path());
    paths.ensure_dirs().unwrap();
    let mut conn = connection::open(paths.db_path()).unwrap();
    let cfg = Config::defaults();

    let refused = {
        let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
        let mut req = request("quality is out of range here");
        req.quality = 9;
        memories::save::run(&mut ctx, req, false, None)
    };
    assert!(refused.is_err(), "quality 9 is refused");

    let recorded = rows(&conn);
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].command, "save");
    assert!(!recorded[0].ok);
    assert_eq!(recorded[0].error_code.as_deref(), Some("bad_request"));
    assert_eq!(recorded[0].summary, None, "a failure carries no summary");

    let memories_written: i64 = conn
        .query_row("SELECT count(*) FROM memories", [], |r| r.get(0))
        .unwrap();
    assert_eq!(memories_written, 0, "the refusal wrote no memory");
}
