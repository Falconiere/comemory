//! Tests for [`comemory::config::paths::Paths`].

use comemory::config::paths::Paths;

#[path = "../common/mod.rs"]
mod common;

#[test]
fn paths_resolves_subdirs_relative_to_data_dir() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());

    assert_eq!(paths.memories_dir(), sb.data_dir().join("memories"));
    assert_eq!(
        paths.trash_dir(),
        sb.data_dir().join("memories").join(".trash")
    );
    // stats_db() is now an alias for db_path() — both land in comemory.db.
    assert_eq!(paths.stats_db(), paths.db_path());
    assert_eq!(paths.config_file(), sb.data_dir().join("config.toml"));
}

#[test]
fn ensure_dirs_creates_full_tree() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().expect("ensure_dirs");

    assert!(paths.memories_dir().exists());
    assert!(paths.trash_dir().exists());
    assert!(paths.index_dir().exists());
}
