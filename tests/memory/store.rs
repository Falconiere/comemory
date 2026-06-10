use comemory::config::paths::Paths;
use comemory::memory::{Kind, MemoryStore, Relations, SaveParams};

#[path = "../common/mod.rs"]
mod common;

/// Note-kind params with the legacy test defaults (`repo = "r"`,
/// `author = "a"`, quality 3) so the simple tests stay one-liners.
fn quick(body: &str) -> SaveParams<'_> {
    SaveParams {
        repo: "r",
        author: "a",
        ..SaveParams::new(body, Kind::Note)
    }
}

#[test]
fn save_then_load_round_trips() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());

    let tags = vec!["postgres".to_string()];
    let rec = store
        .save(SaveParams {
            repo: "qwick-backend",
            tags: &tags,
            author: "falconiere",
            quality: 4,
            ..SaveParams::new("Use Postgres for analytics", Kind::Decision)
        })
        .unwrap();
    assert_eq!(rec.frontmatter.kind, Kind::Decision);
    assert_eq!(rec.frontmatter.tags, vec!["postgres".to_string()]);

    let loaded = store.load(&rec.frontmatter.id).unwrap();
    assert_eq!(loaded.body.trim(), "Use Postgres for analytics");
    assert_eq!(loaded.frontmatter.id, rec.frontmatter.id);
}

#[test]
fn save_writes_relations_into_frontmatter() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());

    let rec = store
        .save(SaveParams {
            relations: Relations {
                supersedes: vec!["a1b2c3d4".to_string()],
                ..Relations::default()
            },
            ..SaveParams::new("new convention replacing an old one", Kind::Convention)
        })
        .unwrap();
    assert_eq!(rec.frontmatter.relations.supersedes, vec!["a1b2c3d4"]);

    // The relation must round-trip through the YAML on disk, not just the
    // in-memory record — markdown is the source of truth for rebuild.
    let loaded = store.load(&rec.frontmatter.id).unwrap();
    assert_eq!(loaded.frontmatter.relations.supersedes, vec!["a1b2c3d4"]);
}

#[test]
fn save_is_atomic_under_failure() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let _ = store.save(quick("body")).unwrap();
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
    let _ = store.save(quick("first")).unwrap();
    let _ = store.save(quick("second")).unwrap();
    let all = store.list().unwrap();
    assert_eq!(all.len(), 2);
}

#[test]
fn delete_removes_file_and_returns_record() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let rec = store.save(quick("to delete")).unwrap();
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

    let _ = store.save(quick("alpha")).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    let _ = store.save(quick("beta")).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    let _ = store.save(quick("gamma")).unwrap();

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

    let good = store.save(quick("valid memory")).unwrap();

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
