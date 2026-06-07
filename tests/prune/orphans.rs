use comemory::config::paths::Paths;
use comemory::memory::{Kind, MemoryStore};
use comemory::prune::orphans;

use super::common;

#[test]
fn no_orphans_on_fresh_dir() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    assert!(orphans::detect(&paths).unwrap().is_empty());
}

#[test]
fn trashed_memory_shows_up_as_orphan() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());

    let rec = store
        .save("to be trashed", Kind::Note, "r", &[], "a", 3)
        .unwrap();
    let _ = store.delete(&rec.frontmatter.id).unwrap();

    let orphan_ids = orphans::detect(&paths).unwrap();
    assert_eq!(
        orphan_ids,
        vec![rec.frontmatter.id.clone()],
        "the trashed memory's id should appear as an orphan (it has no live counterpart)",
    );
}

#[test]
fn live_memory_does_not_appear_as_orphan() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().unwrap();
    let store = MemoryStore::new(paths.clone());
    let _ = store
        .save("still alive", Kind::Note, "r", &[], "a", 3)
        .unwrap();

    // No trash entries exist — orphans must be empty.
    assert!(orphans::detect(&paths).unwrap().is_empty());
}
