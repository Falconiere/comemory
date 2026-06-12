//! Repo-root resolution: `--root` overrides beat the stored
//! `repo_marker.root_path`, which beats an error; and `id_to_abs_path`
//! turns a `file:<repo>:<path>` id into a contained absolute path.

use comemory::errors::Error;
use comemory::serve::repo_root::{RootOverrides, id_to_abs_path, rel_of, resolve_root};
use comemory::store::{code_row, connection};
use tempfile::TempDir;

/// Open a migrated `comemory.db` in a fresh temp data dir.
fn open_db(home: &TempDir) -> rusqlite::Connection {
    connection::open(home.path().join("comemory.db")).expect("open db")
}

#[test]
fn resolve_root_prefers_stored_then_errors() {
    let home = TempDir::new().unwrap();
    let conn = open_db(&home);

    // Unknown repo with no override → BadRequest pointing at --root.
    let err = resolve_root(&conn, "demo", &RootOverrides::new()).unwrap_err();
    assert!(matches!(err, Error::BadRequest(_)), "got {err:?}");

    // Stored root resolves to the canonical directory.
    let root_dir = TempDir::new().unwrap();
    let root = root_dir.path().canonicalize().unwrap();
    code_row::upsert_repo_root(&conn, "demo", &root.to_string_lossy()).unwrap();
    let resolved = resolve_root(&conn, "demo", &RootOverrides::new()).unwrap();
    assert_eq!(resolved, root);
}

#[test]
fn resolve_root_override_wins() {
    let home = TempDir::new().unwrap();
    let conn = open_db(&home);
    let stored_dir = TempDir::new().unwrap();
    code_row::upsert_repo_root(
        &conn,
        "demo",
        &stored_dir.path().canonicalize().unwrap().to_string_lossy(),
    )
    .unwrap();

    let override_dir = TempDir::new().unwrap();
    let override_root = override_dir.path().canonicalize().unwrap();
    let mut overrides = RootOverrides::new();
    overrides.insert("demo".into(), override_dir.path().to_path_buf());

    let resolved = resolve_root(&conn, "demo", &overrides).unwrap();
    assert_eq!(
        resolved, override_root,
        "override must win over stored root"
    );
}

#[test]
fn id_to_abs_path_resolves_and_contains() {
    let home = TempDir::new().unwrap();
    let conn = open_db(&home);
    let root_dir = TempDir::new().unwrap();
    let root = root_dir.path().canonicalize().unwrap();
    std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
    code_row::upsert_repo_root(&conn, "demo", &root.to_string_lossy()).unwrap();

    let abs = id_to_abs_path(&conn, "file:demo:a.rs", &RootOverrides::new()).unwrap();
    assert_eq!(abs, root.join("a.rs"));

    // Malformed id → BadRequest.
    assert!(matches!(
        id_to_abs_path(&conn, "not-a-file-id", &RootOverrides::new()).unwrap_err(),
        Error::BadRequest(_)
    ));
    // Traversal id → Forbidden.
    assert!(matches!(
        id_to_abs_path(&conn, "file:demo:../escape", &RootOverrides::new()).unwrap_err(),
        Error::Forbidden(_)
    ));
}

#[test]
fn rel_of_splits_path() {
    assert_eq!(rel_of("file:demo:src/a.rs"), Some("src/a.rs"));
    assert_eq!(rel_of("bogus"), None);
}
