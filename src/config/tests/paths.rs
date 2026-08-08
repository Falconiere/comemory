#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`comemory::config::paths::Paths`].

use comemory::config::paths::Paths;

use crate::test_common as common;

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
    assert_eq!(paths.sources_file(), sb.data_dir().join("sources.toml"));
    assert_eq!(
        paths.sources_lock_file(),
        sb.data_dir().join("sources.toml.lock")
    );
    assert_eq!(
        paths.migration_backup(12),
        sb.data_dir().join("comemory.db.pre-v12.bak")
    );
    assert_eq!(
        paths.migration_lock_file(),
        sb.data_dir().join("comemory.db.migrate.lock")
    );
    assert_eq!(
        paths.rebuild_backup(),
        sb.data_dir().join("comemory.db.pre-rebuild.bak")
    );
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
