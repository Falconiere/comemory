use qwick::config::paths::Paths;
use qwick::memory::{Kind, MemoryStore};

#[path = "../common/mod.rs"]
mod common;

#[test]
fn save_then_load_round_trips() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());

    let rec = store
        .save(
            "Use Postgres for analytics",
            Kind::Decision,
            "qwick-backend",
            &["postgres".into()],
            "falconiere",
            4,
        )
        .unwrap();
    assert_eq!(rec.frontmatter.kind, Kind::Decision);
    assert_eq!(rec.frontmatter.tags, vec!["postgres".to_string()]);

    let loaded = store.load(&rec.frontmatter.id).unwrap();
    assert_eq!(loaded.body.trim(), "Use Postgres for analytics");
    assert_eq!(loaded.frontmatter.id, rec.frontmatter.id);
}

#[test]
fn save_is_atomic_under_failure() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let _ = store.save("body", Kind::Note, "r", &[], "a", 3).unwrap();
    let entries: Vec<_> = std::fs::read_dir(paths.memories_dir())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().into_string().unwrap())
        .filter(|n| n.ends_with(".tmp"))
        .collect();
    assert!(
        entries.is_empty(),
        "no .tmp files should remain: {entries:?}"
    );
}

#[test]
fn list_returns_all_saved() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths);
    let _ = store.save("first", Kind::Note, "r", &[], "a", 3).unwrap();
    let _ = store.save("second", Kind::Note, "r", &[], "a", 3).unwrap();
    let all = store.list().unwrap();
    assert_eq!(all.len(), 2);
}

#[test]
fn delete_removes_file_and_returns_record() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let rec = store
        .save("to delete", Kind::Note, "r", &[], "a", 3)
        .unwrap();
    let removed = store.delete(&rec.frontmatter.id).unwrap();
    assert_eq!(removed.frontmatter.id, rec.frontmatter.id);
    assert!(store.load(&rec.frontmatter.id).is_err());
}
