#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Integration tests for `comemory gc`: the `.trash/` sweep plus learning
//! telemetry retention. Old `retrieval_log` / `feedback_events` rows are
//! evicted past `prune.learning_retention_days`; aggregated `feedback`
//! counters are permanent and must survive every sweep.

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;
use time::{Duration, OffsetDateTime};

/// Build a `comemory` invocation with `COMEMORY_DATA_DIR` rooted at `home`.
fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
}

/// Path of the sandbox `comemory.db`.
fn db_path(home: &TempDir) -> std::path::PathBuf {
    home.path().join(".comemory").join("comemory.db")
}

/// Open the sandbox `comemory.db` read-only for post-hoc assertions.
fn open_db_readonly(home: &TempDir) -> rusqlite::Connection {
    rusqlite::Connection::open_with_flags(db_path(home), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("open db read-only")
}

/// Count rows of `table` matching `where_sql` (no bind params).
fn count(conn: &rusqlite::Connection, table: &str, where_sql: &str) -> i64 {
    conn.query_row(
        &format!("SELECT COUNT(*) FROM {table} WHERE {where_sql}"),
        [],
        |r| r.get(0),
    )
    .expect("count rows")
}

/// Seed the sandbox db with telemetry straddling the retention cutoff,
/// rendered through the same formatter the production writers use
/// (`store::memory_row::iso_format`), plus one permanent `feedback`
/// counter row.
fn seed_telemetry(home: &TempDir) {
    std::fs::create_dir_all(home.path().join(".comemory")).expect("create data dir");
    let conn = comemory::store::connection::open(db_path(home)).expect("open + migrate db");
    let now = OffsetDateTime::now_utc();
    let old = comemory::store::memory_row::iso_format(now - Duration::days(100))
        .expect("format old stamp");
    let fresh = comemory::store::memory_row::iso_format(now - Duration::days(1))
        .expect("format fresh stamp");
    // gc compares `at < cutoff` lexicographically; that is only chronological
    // if the writer format is fixed-width. Pin the shape here so a format
    // change in iso_format breaks this test rather than silently breaking gc.
    for stamp in [&old, &fresh] {
        assert_eq!(
            stamp.len(),
            30,
            "iso_format must stay fixed-width (YYYY-MM-DDTHH:MM:SS.nnnnnnnnnZ) \
             for lexicographic comparison: {stamp}"
        );
        assert!(
            stamp.ends_with('Z') && stamp.as_bytes()[19] == b'.',
            "iso_format must keep 9 fractional digits + Z suffix: {stamp}"
        );
    }
    for (qid, at) in [("q-old", &old), ("q-new", &fresh)] {
        conn.execute(
            "INSERT INTO retrieval_log(query_id, query, returned_ids, at) \
             VALUES (?1, 'some query', '[]', ?2)",
            rusqlite::params![qid, at],
        )
        .expect("insert retrieval_log row");
        conn.execute(
            "INSERT INTO feedback_events(query_id, memory_id, verdict, at) \
             VALUES (?1, 'aaaaaaaa', 'used', ?2)",
            rusqlite::params![qid, at],
        )
        .expect("insert feedback_events row");
    }
    conn.execute(
        "INSERT INTO feedback(memory_id, used_count, irrelevant_count, last_used) \
         VALUES ('aaaaaaaa', 3, 1, ?1)",
        [&old],
    )
    .expect("insert feedback counter row");
}

#[test]
fn gc_evicts_old_telemetry_keeps_fresh_rows_and_counters() {
    let home = TempDir::new().expect("tempdir");
    seed_telemetry(&home);

    let assert = bin(&home).args(["--json", "gc"]).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let v: Value = serde_json::from_str(stdout.trim()).expect("parse JSON envelope");
    assert_eq!(v["removed"].as_u64(), Some(0), "no trash seeded: {v}");
    assert_eq!(v["log_rows"].as_u64(), Some(1), "one old log row: {v}");
    assert_eq!(v["event_rows"].as_u64(), Some(1), "one old event row: {v}");

    let conn = open_db_readonly(&home);
    assert_eq!(count(&conn, "retrieval_log", "1=1"), 1);
    assert_eq!(count(&conn, "retrieval_log", "query_id = 'q-new'"), 1);
    assert_eq!(count(&conn, "feedback_events", "1=1"), 1);
    assert_eq!(count(&conn, "feedback_events", "query_id = 'q-new'"), 1);
    // Aggregated counters are permanent — even with an ancient last_used.
    assert_eq!(
        count(
            &conn,
            "feedback",
            "memory_id = 'aaaaaaaa' AND used_count = 3"
        ),
        1,
        "feedback counters must survive gc untouched"
    );
}

#[test]
fn gc_sweeps_old_trash_and_keeps_fresh_trash() {
    let home = TempDir::new().expect("tempdir");
    let trash = home
        .path()
        .join(".comemory")
        .join("memories")
        .join(".trash");
    std::fs::create_dir_all(&trash).expect("create trash dir");
    let old = trash.join("11111111-old.md");
    let fresh = trash.join("22222222-fresh.md");
    std::fs::write(&old, "old").expect("write old trash entry");
    std::fs::write(&fresh, "fresh").expect("write fresh trash entry");
    let f = std::fs::OpenOptions::new()
        .write(true)
        .open(&old)
        .expect("reopen old trash entry");
    f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_hours(744))
        .expect("backdate mtime");

    let assert = bin(&home).args(["--json", "gc"]).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let v: Value = serde_json::from_str(stdout.trim()).expect("parse JSON envelope");
    assert_eq!(v["removed"].as_u64(), Some(1), "old trash entry swept: {v}");
    assert!(!old.exists(), "old trash entry must be deleted");
    assert!(fresh.exists(), "fresh trash entry must be kept");
}

#[test]
fn gc_reports_bytes_freed_and_writes_a_gc_runs_row() {
    // AC-32: `comemory gc --json` reports `bytes_freed` equal to the size of
    // the files it actually removed, and writes one `gc_runs` row whose `at`
    // is readable — driven through real soft-deletes (`comemory delete`),
    // aged past the 30-day trash retention with a real mtime rewrite, no
    // faked clock.
    let home = TempDir::new().expect("tempdir");
    let assert = bin(&home)
        .args([
            "--json",
            "save",
            "will be deleted then gc'd",
            "--kind",
            "note",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let saved: Value = serde_json::from_str(stdout.trim()).expect("parse save JSON");
    let id = saved["id"].as_str().expect("save emits id").to_string();

    bin(&home).args(["delete", &id]).assert().success();

    let trash_dir = home
        .path()
        .join(".comemory")
        .join("memories")
        .join(".trash");
    let trashed: Vec<std::path::PathBuf> = std::fs::read_dir(&trash_dir)
        .expect("read .trash")
        .filter_map(std::result::Result::ok)
        .map(|e| e.path())
        .collect();
    assert_eq!(trashed.len(), 1, "one real soft-delete in .trash/");
    let expected_bytes: u64 = std::fs::metadata(&trashed[0])
        .expect("stat trashed file")
        .len();

    // Age the trash file past the 30-day retention with a real mtime
    // rewrite — never a faked clock.
    let f = std::fs::OpenOptions::new()
        .write(true)
        .open(&trashed[0])
        .expect("reopen trashed file");
    f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_hours(31 * 24))
        .expect("backdate mtime");
    drop(f);

    let assert = bin(&home).args(["--json", "gc"]).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let v: Value = serde_json::from_str(stdout.trim()).expect("parse JSON envelope");
    assert_eq!(v["removed"].as_u64(), Some(1));
    assert_eq!(
        v["bytes_freed"].as_u64(),
        Some(expected_bytes),
        "bytes_freed must equal the real trashed file's size: {v}"
    );
    assert!(!trashed[0].exists(), "aged trash entry must be removed");

    let conn = open_db_readonly(&home);
    let (at, removed, bytes_freed): (String, i64, i64) = conn
        .query_row(
            "SELECT at, removed, bytes_freed FROM gc_runs ORDER BY at DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("gc_runs row written by this run");
    assert!(!at.is_empty(), "at must be a readable timestamp");
    time::OffsetDateTime::parse(&at, &time::format_description::well_known::Rfc3339)
        .expect("at must parse as RFC 3339");
    assert_eq!(removed, 1);
    assert_eq!(
        bytes_freed,
        i64::try_from(expected_bytes).expect("fits i64")
    );
}

#[test]
fn gc_on_fresh_dir_does_not_create_db() {
    let home = TempDir::new().expect("tempdir");
    let assert = bin(&home).args(["--json", "gc"]).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let v: Value = serde_json::from_str(stdout.trim()).expect("parse JSON envelope");
    assert_eq!(v["removed"].as_u64(), Some(0));
    assert_eq!(v["log_rows"].as_u64(), Some(0));
    assert_eq!(v["event_rows"].as_u64(), Some(0));
    assert!(
        !db_path(&home).exists(),
        "gc on a fresh dir must not create comemory.db"
    );
}

#[test]
fn gc_tty_summary_reports_all_three_counts() {
    let home = TempDir::new().expect("tempdir");
    seed_telemetry(&home);
    let assert = bin(&home).arg("gc").assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    assert!(
        stdout.contains("0 trashed memories") && stdout.contains("1 log row"),
        "TTY summary must report trash + telemetry counts: {stdout:?}"
    );
    assert!(
        stdout.contains("1 feedback event"),
        "TTY summary must report evicted feedback events: {stdout:?}"
    );
}

#[test]
fn gc_respects_env_retention_override() {
    // With a 200-day window, even the now-100d rows survive.
    let home = TempDir::new().expect("tempdir");
    seed_telemetry(&home);
    let assert = bin(&home)
        .env("COMEMORY_LEARNING_RETENTION_DAYS", "200")
        .args(["--json", "gc"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let v: Value = serde_json::from_str(stdout.trim()).expect("parse JSON envelope");
    assert_eq!(
        v["log_rows"].as_u64(),
        Some(0),
        "wide window keeps all: {v}"
    );
    assert_eq!(v["event_rows"].as_u64(), Some(0));
    let conn = open_db_readonly(&home);
    assert_eq!(count(&conn, "retrieval_log", "1=1"), 2);
    assert_eq!(count(&conn, "feedback_events", "1=1"), 2);
}

#[test]
fn gc_errors_on_invalid_config() {
    // gc now loads the layered config like every other subcommand, so an
    // invalid retention value aborts instead of running with a bad window.
    let home = TempDir::new().expect("tempdir");
    let data = home.path().join(".comemory");
    std::fs::create_dir_all(&data).expect("create data dir");
    std::fs::write(
        data.join("config.toml"),
        "[prune]\nlearning_retention_days = 0\n",
    )
    .expect("write config.toml");
    let assert = bin(&home).arg("gc").assert().failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).expect("utf8 stderr");
    assert!(
        stderr.contains("learning_retention_days"),
        "error must name the offending knob: {stderr:?}"
    );
}
