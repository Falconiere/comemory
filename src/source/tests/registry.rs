#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Test mirror for `src/source/registry.rs`.

use std::fs;

use comemory::config::paths::Paths;
use comemory::source::SourceKind;
use comemory::source::registry::Registry;
use tempfile::TempDir;

use crate::test_common as common;

/// A registry rooted at a fresh sandbox data dir, plus the sandbox's own
/// temp root (kept alive) for creating real source fixtures alongside it.
fn fresh_registry() -> (Registry, TempDir) {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().expect("ensure_dirs");
    (Registry::new(paths), sb.root)
}

#[test]
fn save_then_load_round_trips_entries_exactly() {
    let (reg, root) = fresh_registry();
    let file_src = root.path().join("notes.txt");
    fs::write(&file_src, "hello").expect("seed file source");
    let dir_src = root.path().join("docs");
    fs::create_dir(&dir_src).expect("seed dir source");

    let a = reg
        .register(&file_src, Some("repo-a".to_string()))
        .expect("register file");
    let b = reg.register(&dir_src, None).expect("register dir");

    let loaded = reg.load().expect("load");
    assert_eq!(loaded.len(), 2);
    assert!(
        loaded.contains(&a),
        "loaded set must contain the file entry exactly"
    );
    assert!(
        loaded.contains(&b),
        "loaded set must contain the dir entry exactly"
    );
    assert_eq!(a.kind, SourceKind::File);
    assert_eq!(b.kind, SourceKind::Dir);
}

#[test]
fn load_on_missing_file_is_empty() {
    let (reg, _root) = fresh_registry();
    assert_eq!(reg.load().expect("load"), Vec::new());
}

#[test]
fn overlap_rejects_parent_then_child() {
    let (reg, root) = fresh_registry();
    let parent = root.path().join("repo");
    fs::create_dir(&parent).expect("seed parent dir");
    reg.register(&parent, None).expect("register parent");

    let child = parent.join("child.md");
    fs::write(&child, "x").expect("seed child file");
    let err = reg.register(&child, None).unwrap_err();
    assert!(format!("{err}").contains("overlaps"), "got: {err}");
}

#[test]
fn overlap_rejects_child_then_parent() {
    let (reg, root) = fresh_registry();
    let parent = root.path().join("repo");
    fs::create_dir(&parent).expect("seed parent dir");
    let child = parent.join("child.md");
    fs::write(&child, "x").expect("seed child file");
    reg.register(&child, None).expect("register child first");

    let err = reg.register(&parent, None).unwrap_err();
    assert!(format!("{err}").contains("overlaps"), "got: {err}");
}

#[test]
fn reregistering_same_path_updates_not_duplicates() {
    let (reg, root) = fresh_registry();
    let src = root.path().join("docs");
    fs::create_dir(&src).expect("seed dir");

    let first = reg
        .register(&src, Some("repo-1".to_string()))
        .expect("first");
    let second = reg
        .register(&src, Some("repo-2".to_string()))
        .expect("second");

    assert_eq!(
        first.id, second.id,
        "id must be stable across re-registration"
    );
    assert_eq!(second.repo.as_deref(), Some("repo-2"));
    assert_eq!(
        first.created_at, second.created_at,
        "created_at must not move"
    );
    assert_eq!(
        reg.load().expect("load").len(),
        1,
        "must not duplicate the row"
    );
}

#[test]
fn symlinked_alias_resolves_to_the_same_entry_not_a_duplicate() {
    let (reg, root) = fresh_registry();
    let real = root.path().join("real-docs");
    fs::create_dir(&real).expect("seed real dir");
    let alias = root.path().join("alias-docs");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&real, &alias).expect("seed symlink");
    #[cfg(unix)]
    {
        let via_real = reg
            .register(&real, Some("via-real".to_string()))
            .expect("register real");
        let via_alias = reg
            .register(&alias, Some("via-alias".to_string()))
            .expect("register alias must update, not conflict");
        assert_eq!(
            via_real.id, via_alias.id,
            "symlink alias must resolve to same id"
        );
        assert_eq!(reg.load().expect("load").len(), 1);
    }
}

#[test]
fn unregister_removes_by_id_and_by_path() {
    let (reg, root) = fresh_registry();
    let a = root.path().join("a.txt");
    fs::write(&a, "x").expect("seed a");
    let b = root.path().join("b.txt");
    fs::write(&b, "x").expect("seed b");

    let entry_a = reg.register(&a, None).expect("register a");
    reg.register(&b, None).expect("register b");

    let removed = reg
        .unregister(entry_a.id.as_str())
        .expect("unregister by id")
        .expect("must find entry a");
    assert_eq!(removed.id, entry_a.id);

    let removed_b = reg
        .unregister(b.to_string_lossy().as_ref())
        .expect("unregister by path")
        .expect("must find entry b");
    assert_eq!(
        removed_b.canonical_path,
        fs::canonicalize(&b).expect("canonicalize b")
    );

    assert_eq!(reg.load().expect("load"), Vec::new());
}

#[test]
fn unregister_unknown_target_returns_none() {
    let (reg, _root) = fresh_registry();
    assert_eq!(
        reg.unregister("deadbeefdeadbeefdeadbeefdeadbeef")
            .expect("unregister"),
        None
    );
}
