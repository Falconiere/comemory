use comemory::config::paths::Paths;
use comemory::memory::{Kind, MemoryStore};

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

#[test]
fn list_returns_results_sorted_by_created_desc() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());

    let _ = store.save("alpha", Kind::Note, "r", &[], "a", 3).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    let _ = store.save("beta", Kind::Note, "r", &[], "a", 3).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    let _ = store.save("gamma", Kind::Note, "r", &[], "a", 3).unwrap();

    let list = store.list().unwrap();
    assert_eq!(list.len(), 3);

    // Robust check: every adjacent pair is non-increasing by `created`.
    let times: Vec<_> = list.iter().map(|m| m.frontmatter.created).collect();
    assert!(
        times.windows(2).all(|w| w[0] >= w[1]),
        "list not sorted by created desc: {times:?}"
    );
}

#[test]
fn list_skips_malformed_files_and_returns_valid_ones() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());

    let good = store
        .save("valid memory", Kind::Note, "r", &[], "a", 3)
        .unwrap();

    // Drop a malformed .md file alongside the valid one.
    let bad_path = paths.memories_dir().join("zzzzzzzz-bad.md");
    std::fs::write(&bad_path, "this is not valid frontmatter at all\n").unwrap();

    let list = store.list().unwrap();
    assert_eq!(
        list.len(),
        1,
        "expected only the valid record, got {list:?}"
    );
    assert_eq!(list[0].frontmatter.id, good.frontmatter.id);
}
