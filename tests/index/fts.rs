use comemory::config::paths::Paths;
use comemory::index::Fts;

use super::common;

#[test]
fn open_creates_db_and_table() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let _fts = Fts::open(paths.index_dir().join("fts.sqlite")).unwrap();
    assert!(paths.index_dir().join("fts.sqlite").exists());
}

#[test]
fn upsert_then_count_returns_one() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let fts = Fts::open(paths.index_dir().join("fts.sqlite")).unwrap();
    fts.upsert("a1b2c3d4", "Use Postgres for analytics")
        .unwrap();
    assert_eq!(fts.count().unwrap(), 1);
}

#[test]
fn upsert_same_id_overwrites() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let fts = Fts::open(paths.index_dir().join("fts.sqlite")).unwrap();
    fts.upsert("a1b2c3d4", "first body").unwrap();
    fts.upsert("a1b2c3d4", "second body").unwrap();
    assert_eq!(fts.count().unwrap(), 1);
}

#[test]
fn delete_removes_row() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let fts = Fts::open(paths.index_dir().join("fts.sqlite")).unwrap();
    fts.upsert("a1b2c3d4", "body").unwrap();
    fts.delete("a1b2c3d4").unwrap();
    assert_eq!(fts.count().unwrap(), 0);
}
