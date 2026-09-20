#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The activity feed as a real `comemory` process writes it: what a terminal
//! run records, the `COMEMORY_ACTOR` label it may carry, the two config knobs
//! that narrow or silence it, and the guarantee that a broken feed never
//! breaks the command.
//!
//! The env layer is covered here rather than in `src/config/tests/activity.rs`
//! because a child process takes its environment as an argument — no
//! `set_var`, no process-global mutation, and the evidence is a real run.

#[path = "common/git_commit.rs"]
mod git_commit;
#[path = "common/git_repo.rs"]
mod git_repo;

use assert_cmd::Command;
use comemory::store::activity::{ActivityFilter, ActivityRow, list};
use comemory::store::connection;
use serde_json::Value;
use tempfile::TempDir;

/// A `comemory` invocation pointed at `home`'s data dir.
fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
}

fn data_dir(home: &TempDir) -> std::path::PathBuf {
    home.path().join(".comemory")
}

/// Every recorded row, newest first.
fn rows(home: &TempDir) -> Vec<ActivityRow> {
    let conn = connection::open(data_dir(home).join("comemory.db")).expect("open db");
    list(&conn, &ActivityFilter::default(), 0, 0)
        .expect("list activity")
        .0
}

fn summary_of(row: &ActivityRow) -> Value {
    serde_json::from_str(row.summary.as_deref().expect("a summary")).expect("summary json")
}

/// Save one memory, returning nothing — the row it records is the subject.
fn save(home: &TempDir, body: &str) {
    bin(home)
        .args(["save", body, "--repo", "demo", "--kind", "decision"])
        .assert()
        .success();
}

#[test]
fn a_terminal_save_records_one_cli_row_with_its_summary() {
    let home = TempDir::new().unwrap();
    save(&home, "use pgbouncer in transaction mode");

    let recorded = rows(&home);
    assert_eq!(recorded.len(), 1, "one row for one run");
    assert_eq!(recorded[0].command, "save");
    assert_eq!(recorded[0].source, "cli");
    assert_eq!(recorded[0].repo.as_deref(), Some("demo"));
    assert!(recorded[0].ok);
    assert_eq!(
        recorded[0].actor, None,
        "no COMEMORY_ACTOR was set, so no actor is claimed"
    );
    assert_eq!(summary_of(&recorded[0])["kind"], "decision");
}

#[test]
fn comemory_actor_labels_the_run_and_is_trimmed() {
    let home = TempDir::new().unwrap();
    bin(&home)
        .env("COMEMORY_ACTOR", "  claude-code/2.1.0  ")
        .args(["save", "label this run", "--repo", "demo"])
        .assert()
        .success();

    let recorded = rows(&home);
    assert_eq!(recorded[0].actor.as_deref(), Some("claude-code/2.1.0"));
}

#[test]
fn a_blank_actor_records_no_actor() {
    let home = TempDir::new().unwrap();
    bin(&home)
        .env("COMEMORY_ACTOR", "   ")
        .args(["save", "blank actor", "--repo", "demo"])
        .assert()
        .success();

    assert_eq!(rows(&home)[0].actor, None);
}

#[test]
fn summaries_off_records_the_run_without_its_detail() {
    let home = TempDir::new().unwrap();
    bin(&home)
        .env("COMEMORY_ACTIVITY_SUMMARIES", "false")
        .args(["save", "a body worth hiding", "--repo", "demo"])
        .assert()
        .success();

    let recorded = rows(&home);
    assert_eq!(recorded.len(), 1, "the run is still recorded");
    assert_eq!(recorded[0].command, "save");
    assert_eq!(recorded[0].summary, None, "no per-command detail is stored");
}

#[test]
fn a_disabled_feed_records_nothing() {
    let home = TempDir::new().unwrap();
    bin(&home)
        .env("COMEMORY_ACTIVITY_ENABLED", "false")
        .args(["save", "not recorded", "--repo", "demo"])
        .assert()
        .success();

    assert!(rows(&home).is_empty());
}

#[test]
fn a_missing_activity_table_never_fails_the_command() {
    let home = TempDir::new().unwrap();
    save(&home, "the first save creates the database");

    // Drop the feed's table out from under the next run: `store::migrate` is
    // keyed by `schema_meta` markers, so opening the database does not put it
    // back, and the writer meets a real missing-table error.
    let conn = connection::open(data_dir(&home).join("comemory.db")).unwrap();
    conn.execute_batch("DROP TABLE activity_log;").unwrap();
    drop(conn);

    bin(&home)
        .args(["save", "the second save still succeeds", "--repo", "demo"])
        .assert()
        .success();

    let conn = connection::open(data_dir(&home).join("comemory.db")).unwrap();
    let saved: i64 = conn
        .query_row("SELECT count(*) FROM memories", [], |r| r.get(0))
        .unwrap();
    assert_eq!(saved, 2, "both memories are on record");
}

#[test]
fn a_terminal_find_records_its_query_and_total_hit_count() {
    let home = TempDir::new().unwrap();
    save(&home, "pgbouncer in transaction mode fixes pool exhaustion");

    let out = bin(&home)
        .args(["find", "pgbouncer", "--json"])
        .assert()
        .success();
    let printed: Value =
        serde_json::from_slice(&out.get_output().stdout).expect("find --json output");
    let reported = printed["hits"].as_array().expect("hits array").len();

    let recorded = rows(&home);
    let find_row = recorded
        .iter()
        .find(|r| r.command == "find")
        .expect("a find row");
    assert_eq!(find_row.source, "cli");
    let summary = summary_of(find_row);
    assert_eq!(summary["query"], "pgbouncer");
    assert_eq!(
        summary["total"].as_u64().expect("a total"),
        reported as u64,
        "the row reports the hit count the command printed"
    );
}

#[test]
fn an_index_code_run_records_the_repo_and_the_files_it_indexed() {
    let home = TempDir::new().unwrap();
    let repo = TempDir::new().unwrap();
    git_repo::init_repo(repo.path());
    git_commit::commit_files(
        repo.path(),
        &[
            ("src/lib.rs", "pub fn pool_size() -> usize { 42 }\n"),
            ("src/other.rs", "pub fn other() -> usize { 7 }\n"),
        ],
        "seed",
    );

    bin(&home)
        .args([
            "index-code",
            "--repo",
            "demo",
            "--path",
            repo.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    // `index-code` prints nothing on a non-TTY success, so the count is
    // checked against the store it actually wrote.
    let conn = connection::open(data_dir(&home).join("comemory.db")).unwrap();
    let indexed_files: i64 = conn
        .query_row(
            "SELECT count(DISTINCT path) FROM code_symbols WHERE repo = 'demo'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(indexed_files, 2, "both files were indexed");

    let recorded = rows(&home);
    let row = recorded
        .iter()
        .find(|r| r.command == "index-code")
        .expect("an index-code row");
    assert_eq!(row.repo.as_deref(), Some("demo"));
    let summary = summary_of(row);
    assert_eq!(summary["repo"], "demo");
    assert_eq!(
        summary["files"].as_i64().expect("a file count"),
        indexed_files,
        "the row reports the files the run actually indexed"
    );
}
