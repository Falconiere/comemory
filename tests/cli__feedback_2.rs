//! Integration tests for `comemory feedback` — part 2.
//!
//! Covers: vanished/invalid code-symbol id rejection, mixed memory+code
//! flag flows, and the full save → search → feedback provenance join.

#[path = "common/cli_feedback_support.rs"]
mod support;

use serde_json::Value;
use support::{bin, open_db_readonly, run_json, seed_code_symbol};
use tempfile::TempDir;

/// Extract a required string field from a JSON envelope.
fn json_str(v: &Value, field: &str) -> String {
    v.get(field)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("envelope field {field:?} missing in {v}"))
        .to_string()
}

#[test]
fn feedback_errors_loudly_on_vanished_code_symbol_id() {
    // A well-formed symbol id that names no live `code_symbols` row must
    // fail loudly (unlike the query-id warn path): the rowid may already
    // belong to an unrelated symbol after a re-index, so recording it
    // would misattribute the verdict.
    let home = TempDir::new().expect("tempdir");
    // Create the db (and one symbol) so the failure is the missing id,
    // not a missing table.
    let live = seed_code_symbol(&home, "demo", "a.rs", "alpha");
    let dead = live + 1_000;
    let assertion = bin(&home)
        .args([
            "feedback",
            "q-20260610-aabbccdd",
            "--used-code",
            &dead.to_string(),
        ])
        .assert()
        .failure();
    let stderr = String::from_utf8(assertion.get_output().stderr.clone()).expect("utf8 stderr");
    assert!(
        stderr.contains(&dead.to_string()),
        "stderr should name the vanished id, got: {stderr:?}"
    );
    let conn = open_db_readonly(&home);
    let rows: i64 = conn
        .query_row("SELECT count(*) FROM code_feedback", [], |r| r.get(0))
        .expect("count");
    assert_eq!(rows, 0, "nothing may be recorded for a vanished id");
}

#[test]
fn feedback_rejects_non_positive_and_non_numeric_symbol_ids() {
    // Symbol ids are positive integers (code_symbols rowids): zero,
    // negatives, and non-numeric input must fail loudly, naming the flag,
    // before anything is written.
    let home = TempDir::new().expect("tempdir");
    for bad in ["0", "-5", "abc"] {
        let assertion = bin(&home)
            .args([
                "feedback",
                "q-20260610-aabbccdd",
                &format!("--used-code={bad}"),
            ])
            .assert()
            .failure();
        let stderr = String::from_utf8(assertion.get_output().stderr.clone()).expect("utf8 stderr");
        assert!(
            stderr.contains("--used-code") && stderr.contains(bad),
            "stderr should name the flag and the bad id {bad:?}, got: {stderr:?}"
        );
    }
    let db_path = home.path().join(".comemory").join("comemory.db");
    assert!(
        !db_path.exists(),
        "rejected symbol ids must not create or write the db"
    );
}

#[test]
fn feedback_mixed_memory_and_code_flags_write_all_four_kinds() {
    // One invocation carrying all four flags records every verdict: two
    // memory-tagged events + the `feedback` counters, two code-tagged
    // events + the `code_feedback` counters.
    let home = TempDir::new().expect("tempdir");
    let used_code = seed_code_symbol(&home, "demo", "a.rs", "alpha");
    let irrelevant_code = seed_code_symbol(&home, "demo", "b.rs", "beta");
    let v = run_json(
        &home,
        &[
            "feedback",
            "q-20260610-aabbccdd",
            "--used",
            "a1b2c3d4",
            "--irrelevant",
            "cccc0003",
            "--used-code",
            &used_code.to_string(),
            "--irrelevant-code",
            &irrelevant_code.to_string(),
        ],
    );
    assert_eq!(v["used"].as_u64(), Some(1));
    assert_eq!(v["irrelevant"].as_u64(), Some(1));
    assert_eq!(v["used_code"].as_u64(), Some(1));
    assert_eq!(v["irrelevant_code"].as_u64(), Some(1));

    let conn = open_db_readonly(&home);
    let (memory_events, code_events): (i64, i64) = conn
        .query_row(
            "SELECT (SELECT count(*) FROM feedback_events
                      WHERE query_id = 'q-20260610-aabbccdd' AND target_kind = 'memory'),
                    (SELECT count(*) FROM feedback_events
                      WHERE query_id = 'q-20260610-aabbccdd' AND target_kind = 'code')",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("event counts");
    assert_eq!(memory_events, 2, "used + irrelevant memory events");
    assert_eq!(code_events, 2, "used + irrelevant code events");
    let (mem_used, code_used): (i64, i64) = conn
        .query_row(
            "SELECT (SELECT used_count FROM feedback WHERE memory_id = 'a1b2c3d4'),
                    (SELECT used_count FROM code_feedback
                      WHERE repo = 'demo' AND path = 'a.rs' AND symbol = 'alpha')",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("counter rows");
    assert_eq!(mem_used, 1);
    assert_eq!(code_used, 1);
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

    // Known-query TTY ack stays a plain "ok" — the orphan notice is
    // reserved for ids missing from retrieval_log.
    let ack = bin(&home)
        .args(["feedback", &query_id, "--used", &memory_id])
        .assert()
        .success();
    let stdout = String::from_utf8(ack.get_output().stdout.clone()).expect("utf8 stdout");
    assert_eq!(
        stdout.trim(),
        "ok",
        "known query id must ack with a bare ok"
    );
}
