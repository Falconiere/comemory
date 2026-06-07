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
