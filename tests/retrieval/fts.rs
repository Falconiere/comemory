use comemory::config::paths::Paths;
use comemory::index::Fts;
use comemory::retrieval::fts::search_fts_ids;

use super::common;

#[test]
fn search_fts_ids_returns_bm25_ordered_ids() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let db = paths.fts_db();
    let fts = Fts::open(&db).unwrap();
    fts.upsert("id_match", "postgres analytics").unwrap();
    fts.upsert("id_miss", "redis cache").unwrap();
    drop(fts);

    let ids = search_fts_ids(&db, "postgres", 5).unwrap();
    assert_eq!(ids.first().map(String::as_str), Some("id_match"));
    assert!(!ids.iter().any(|x| x == "id_miss"));
}

#[test]
fn search_fts_ids_returns_empty_when_db_missing() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let ids = search_fts_ids(paths.index_dir().join("missing.sqlite"), "q", 5).unwrap();
    assert!(ids.is_empty());
}
