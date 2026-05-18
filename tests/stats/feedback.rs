use qwick::stats::feedback::Feedback;
use qwick::stats::sqlite::StatsDb;

use super::common;

#[test]
fn record_used_increments_count() {
    let sb = common::runner::Sandbox::new();
    let mut db = StatsDb::open(sb.data_dir().join("stats.db")).unwrap();
    let fb = Feedback::new(&mut db);
    fb.record_used("m1").unwrap();
    fb.record_used("m1").unwrap();
    let (used, irrelevant) = fb.counts("m1").unwrap();
    assert_eq!(used, 2);
    assert_eq!(irrelevant, 0);
}

#[test]
fn record_irrelevant_increments_count() {
    let sb = common::runner::Sandbox::new();
    let mut db = StatsDb::open(sb.data_dir().join("stats.db")).unwrap();
    let fb = Feedback::new(&mut db);
    fb.record_irrelevant("m2").unwrap();
    let (used, irrelevant) = fb.counts("m2").unwrap();
    assert_eq!(used, 0);
    assert_eq!(irrelevant, 1);
}

#[test]
fn counts_returns_error_on_schema_drift() {
    // Schema drift: if the `feedback` table is missing entirely, `counts`
    // must surface the SQLite error rather than swallow it as (0, 0). The
    // previous implementation used `unwrap_or((0, 0))` which masked any
    // failure mode that wasn't `QueryReturnedNoRows`.
    let sb = common::runner::Sandbox::new();
    let mut db = StatsDb::open(sb.data_dir().join("stats.db")).unwrap();
    db.conn()
        .execute("DROP TABLE feedback", [])
        .expect("drop feedback table");

    let fb = Feedback::new(&mut db);
    let result = fb.counts("anything");
    let err = result.expect_err("counts must error when feedback table is missing");
    let msg = err.to_string();
    assert!(
        msg.contains("feedback"),
        "error should mention 'feedback', got: {msg}"
    );
}
