//! Integration tests for `comemory feedback`: query-id validation, the
//! provenance write into `feedback_events`, and the legacy counter table.
//!
//! Part 1: memory-id validation, JSON envelope, dedup, query-id shape, and
//! the code-target happy path.

#[path = "common/cli_feedback_support.rs"]
mod support;

use support::{bin, open_db_readonly, run_json, seed_code_symbol};
use tempfile::TempDir;

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
    // counters + events are still written. The TTY ack must carry the
    // notice itself — the `tracing::warn!` is invisible at the default
    // EnvFilter level, so a bare "ok" would silently hide the orphan.
    let home = TempDir::new().expect("tempdir");
    bin(&home)
        .args(["feedback", "q-20260610-deadbeef", "--used", "a1b2c3d4"])
        .assert()
        .success()
        .stdout(predicates::str::contains("query id not in log"));

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

#[test]
fn feedback_records_code_target_ids() {
    // `--used-code` / `--irrelevant-code` write `feedback_events` rows with
    // target_kind='code' (symbol id text-encoded into the memory_id column)
    // plus identity-keyed `code_feedback` counter rows, and the JSON ack
    // reports the counts.
    let home = TempDir::new().expect("tempdir");
    let used_id = seed_code_symbol(&home, "demo", "a.rs", "alpha");
    let irrelevant_id = seed_code_symbol(&home, "demo", "b.rs", "beta");
    let v = run_json(
        &home,
        &[
            "feedback",
            "q-20260610-aabbccdd",
            "--used-code",
            &used_id.to_string(),
            "--irrelevant-code",
            &irrelevant_id.to_string(),
        ],
    );
    assert_eq!(v["ok"].as_bool(), Some(true));
    assert_eq!(v["used_code"].as_u64(), Some(1));
    assert_eq!(v["irrelevant_code"].as_u64(), Some(1));

    let conn = open_db_readonly(&home);
    let (code_events, used_count, irrelevant_count): (i64, i64, i64) = conn
        .query_row(
            "SELECT (SELECT count(*) FROM feedback_events
                      WHERE query_id = 'q-20260610-aabbccdd' AND target_kind = 'code'),
                    (SELECT used_count FROM code_feedback
                      WHERE repo = 'demo' AND path = 'a.rs' AND symbol = 'alpha'),
                    (SELECT irrelevant_count FROM code_feedback
                      WHERE repo = 'demo' AND path = 'b.rs' AND symbol = 'beta')",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("code feedback rows");
    assert_eq!(code_events, 2, "one code-tagged event per symbol id");
    assert_eq!(used_count, 1);
    assert_eq!(irrelevant_count, 1);
}
