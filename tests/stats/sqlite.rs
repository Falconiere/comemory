use qwick_memory::stats::sqlite::StatsDb;

use super::common;

#[test]
fn open_creates_schema() {
    let sb = common::runner::Sandbox::new();
    let db = StatsDb::open(sb.data_dir().join("stats.db")).unwrap();
    let mut stmt = db
        .conn()
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap();
    let tables: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert!(tables.iter().any(|t| t == "feedback"));
    assert!(tables.iter().any(|t| t == "retrieval_log"));
    assert!(tables.iter().any(|t| t == "repo_marker"));
}
