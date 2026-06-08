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
    assert_eq!(
        paths.vectors_dir(),
        sb.data_dir().join("index").join("vectors.lance")
    );
    assert_eq!(
        paths.graph_dir(),
        sb.data_dir().join("index").join("graph.kuzu")
    );
    // stats_db() is now an alias for db_path() — both land in comemory.db.
    assert_eq!(paths.stats_db(), paths.db_path());
    assert_eq!(paths.config_file(), sb.data_dir().join("config.toml"));
}

#[test]
fn ensure_dirs_creates_full_tree() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();

    assert!(paths.memories_dir().exists());
    assert!(paths.trash_dir().exists());
    assert!(paths.vectors_dir().parent().unwrap().exists());
    assert!(paths.graph_dir().parent().unwrap().exists());
}
