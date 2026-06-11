//! Integration tests for `comemory feedback`: query-id validation, the
//! provenance write into `feedback_events`, and the legacy counter table.
//!
//! Tests moved here from `tests/cli.rs` when Task 8 gave `feedback` its own
//! provenance behavior worth a dedicated file.

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

/// Build a `comemory` invocation with `COMEMORY_DATA_DIR` rooted at `home`.
fn bin(home: &TempDir) -> Command {
    let mut c = Command::cargo_bin("comemory").expect("cargo_bin comemory");
    c.env("COMEMORY_DATA_DIR", home.path().join(".comemory"));
    c
}

/// Open the sandbox `comemory.db` read-only for post-hoc assertions.
fn open_db_readonly(home: &TempDir) -> rusqlite::Connection {
    rusqlite::Connection::open_with_flags(
        home.path().join(".comemory").join("comemory.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("open db read-only")
}

#[test]
fn feedback_records_used_and_irrelevant_ids() {
    let home = TempDir::new().expect("tempdir");
    bin(&home)
        .args([
            "feedback",
            "q-20260610-aabbccdd",
            "--used",
            "aaaa0001,bbbb0002",
            "--irrelevant",
            "cccc0003",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("ok"));
}

#[test]
fn feedback_json_emits_counts_and_query_provenance() {
    // Under `--json`, `feedback` must emit a structured envelope reporting
    // how many used/irrelevant ids were recorded, plus the query id and
    // whether it was found in `retrieval_log` (here: no search ran, so
    // `known_query` must be false).
    let home = TempDir::new().expect("tempdir");
    let v = run_json(
        &home,
        &[
            "feedback",
            "q-20260610-aabbccdd",
            "--used",
            "aaaa0001,bbbb0002",
            "--irrelevant",
            "cccc0003",
        ],
    );
    assert_eq!(v["ok"].as_bool(), Some(true));
    assert_eq!(v["used"].as_u64(), Some(2));
    assert_eq!(v["irrelevant"].as_u64(), Some(1));
    assert_eq!(v["query_id"].as_str(), Some("q-20260610-aabbccdd"));
    assert_eq!(v["known_query"].as_bool(), Some(false));
}

#[test]
fn feedback_deduplicates_repeated_ids() {
    // Fix 9 regression: `--used a,a` used to record twice, double-counting
    // the Beta-feedback posterior. The CSV is de-duplicated now.
    let home = TempDir::new().expect("tempdir");
    let v = run_json(
        &home,
        &[
            "feedback",
            "q-20260610-aabbccdd",
            "--used",
            "a1b2c3d4,a1b2c3d4",
        ],
    );
    assert_eq!(v["used"].as_u64(), Some(1), "duplicate id must count once");

    let conn = open_db_readonly(&home);
    let used_count: i64 = conn
        .query_row(
            "SELECT used_count FROM feedback WHERE memory_id = 'a1b2c3d4'",
            [],
            |r| r.get(0),
        )
        .expect("feedback row");
    assert_eq!(used_count, 1, "DB must record exactly one use");
}

#[test]
fn feedback_rejects_invalid_memory_id() {
    // Fix 9 regression: malformed ids used to write orphan feedback rows
    // that no ranking lookup would ever join. They are rejected up front.
    let home = TempDir::new().expect("tempdir");
    let assertion = bin(&home)
        .args(["feedback", "q-20260610-aabbccdd", "--used", "not-an-id"])
        .assert()
        .failure();
    let stderr = String::from_utf8(assertion.get_output().stderr.clone()).expect("utf8 stderr");
    assert!(
        stderr.contains("--used") && stderr.contains("not-an-id"),
        "stderr should name the flag and the bad id, got: {stderr:?}"
    );
}

#[test]
fn feedback_rejects_malformed_query_id() {
    // Free-form query ids are no longer accepted (spec §2.4): the shape
    // must match what `comemory search` prints, and the error must teach
    // the caller that shape.
    let home = TempDir::new().expect("tempdir");
    let assertion = bin(&home)
        .args(["feedback", "q1", "--used", "a1b2c3d4"])
        .assert()
        .failure();
    let stderr = String::from_utf8(assertion.get_output().stderr.clone()).expect("utf8 stderr");
    assert!(
        stderr.contains("q-<yyyymmdd>-<8hex>") && stderr.contains("q1"),
        "stderr should name the bad id and the expected shape, got: {stderr:?}"
    );

    // Nothing may be recorded on the rejected path — validation fires
    // before the data dir / db are even created.
    let db_path = home.path().join(".comemory").join("comemory.db");
    assert!(
        !db_path.exists(),
        "rejected query id must not create or write the db"
    );
}

#[test]
fn feedback_unknown_query_id_warns_but_records() {
    // A valid-shaped query id absent from `retrieval_log` (evicted by gc,
    // or replayed feedback) is a warning, not an error: exit 0 and the
    // counters + events are still written.
    let home = TempDir::new().expect("tempdir");
    bin(&home)
        .args(["feedback", "q-20260610-deadbeef", "--used", "a1b2c3d4"])
        .assert()
        .success();

    let conn = open_db_readonly(&home);
    let (used_count, events): (i64, i64) = conn
        .query_row(
            "SELECT (SELECT used_count FROM feedback WHERE memory_id = 'a1b2c3d4'),
                    (SELECT count(*) FROM feedback_events
                      WHERE query_id = 'q-20260610-deadbeef' AND verdict = 'used')",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("feedback rows");
    assert_eq!(used_count, 1, "counter must be recorded on the warn path");
    assert_eq!(events, 1, "event must be recorded on the warn path");
}

/// Run a `--json` subcommand to success and parse its stdout envelope.
fn run_json(home: &TempDir, args: &[&str]) -> Value {
    let mut cmd = bin(home);
    cmd.arg("--json").args(args);
    let assert = cmd.assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    serde_json::from_str(stdout.trim()).expect("parse JSON envelope")
}

/// Extract a required string field from a JSON envelope.
fn json_str(v: &Value, field: &str) -> String {
    v.get(field)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("envelope field {field:?} missing in {v}"))
        .to_string()
}

#[test]
fn feedback_full_flow_links_events_to_retrieval_log() {
    // End-to-end provenance: save -> search --json (emits query_id and a
    // retrieval_log row) -> feedback --used -> the feedback_events row must
    // join back to that retrieval_log row on query_id.
    let home = TempDir::new().expect("tempdir");
    let save = run_json(
        &home,
        &[
            "save",
            "postgres advisory locks for migration ordering",
            "--kind",
            "decision",
        ],
    );
    let memory_id = json_str(&save, "id");
    let search = run_json(&home, &["search", "advisory lock"]);
    let query_id = json_str(&search, "query_id");

    let fb = run_json(&home, &["feedback", &query_id, "--used", &memory_id]);
    assert_eq!(fb["known_query"].as_bool(), Some(true));
    assert_eq!(fb["query_id"].as_str(), Some(query_id.as_str()));

    let conn = open_db_readonly(&home);
    let (events, verdict): (i64, String) = conn
        .query_row(
            "SELECT count(*), max(fe.verdict)
               FROM feedback_events fe
               JOIN retrieval_log rl ON rl.query_id = fe.query_id
              WHERE fe.query_id = ?1 AND fe.memory_id = ?2",
            rusqlite::params![query_id, memory_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("joined provenance row");
    assert_eq!(events, 1, "exactly one event must join retrieval_log");
    assert_eq!(verdict, "used");
}
