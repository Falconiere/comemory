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
