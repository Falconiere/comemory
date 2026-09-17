#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! The canonicalize-and-contain path check every filesystem-touching surface
//! goes through. A bug in `resolve_within` is a write-anywhere vulnerability,
//! so traversal, absolute, NUL, and symlink-escape cases are all asserted
//! against real temp directories and real symlinks.

use comemory::errors::Error;
use comemory::utilities::path_containment::{contain_abs, resolve_within};
use tempfile::TempDir;

#[test]
fn resolve_within_accepts_contained_path() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "fn x() {}\n").unwrap();
    let abs = resolve_within(&root, "src/lib.rs").expect("contained");
    assert_eq!(abs, root.join("src/lib.rs"));
}

#[test]
fn resolve_within_rejects_parent_traversal() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let err = resolve_within(&root, "../escape").unwrap_err();
    assert!(matches!(err, Error::Forbidden(_)), "got {err:?}");
}

#[test]
fn resolve_within_rejects_absolute() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let err = resolve_within(&root, "/etc/passwd").unwrap_err();
    assert!(matches!(err, Error::Forbidden(_)), "got {err:?}");
}

#[test]
fn resolve_within_rejects_nul_and_empty() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    assert!(matches!(
        resolve_within(&root, "a\0b").unwrap_err(),
        Error::Forbidden(_)
    ));
    assert!(matches!(
        resolve_within(&root, "").unwrap_err(),
        Error::BadRequest(_)
    ));
}

#[test]
fn resolve_within_allows_not_yet_existing_file() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let abs = resolve_within(&root, "new_file.rs").expect("new file ok");
    assert_eq!(abs, root.join("new_file.rs"));
}

#[cfg(unix)]
#[test]
fn resolve_within_rejects_symlink_escape() {
    let outside = TempDir::new().unwrap();
    std::fs::write(outside.path().join("secret"), "s").unwrap();
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    // A symlink inside the root pointing at a file outside it.
    std::os::unix::fs::symlink(outside.path().join("secret"), root.join("link")).unwrap();
    let err = resolve_within(&root, "link").unwrap_err();
    assert!(matches!(err, Error::Forbidden(_)), "got {err:?}");
}

#[test]
fn contain_abs_accepts_path_inside_a_root() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();

    let abs = contain_abs(std::slice::from_ref(&root), &root.join("a.rs")).expect("contained");
    assert_eq!(abs, root.join("a.rs"));
}

#[test]
fn contain_abs_rejects_path_outside_every_root() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let outside = TempDir::new().unwrap();
    let target = outside.path().join("secret");
    std::fs::write(&target, "s").unwrap();

    let err = contain_abs(&[root], &target).unwrap_err();
    assert!(matches!(err, Error::Forbidden(_)), "got {err:?}");
}

#[test]
fn contain_abs_rejects_nonexistent_path() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();

    let err = contain_abs(std::slice::from_ref(&root), &root.join("does-not-exist")).unwrap_err();
    assert!(matches!(err, Error::BadRequest(_)), "got {err:?}");
}

#[cfg(unix)]
#[test]
fn contain_abs_rejects_symlink_escape() {
    let outside = TempDir::new().unwrap();
    std::fs::write(outside.path().join("secret"), "s").unwrap();
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::os::unix::fs::symlink(outside.path().join("secret"), root.join("link")).unwrap();

    let err = contain_abs(std::slice::from_ref(&root), &root.join("link")).unwrap_err();
    assert!(matches!(err, Error::Forbidden(_)), "got {err:?}");
}

#[test]
fn contain_abs_accepts_path_inside_the_second_of_several_roots() {
    let first = TempDir::new().unwrap();
    let first_root = first.path().canonicalize().unwrap();
    let second = TempDir::new().unwrap();
    let second_root = second.path().canonicalize().unwrap();
    std::fs::write(second_root.join("b.rs"), "fn b() {}\n").unwrap();

    let abs = contain_abs(
        &[first_root, second_root.clone()],
        &second_root.join("b.rs"),
    )
    .expect("contained in the second root");
    assert_eq!(abs, second_root.join("b.rs"));
}

#[test]
fn contain_abs_with_empty_roots_always_forbids() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();

    let err = contain_abs(&[], &root.join("a.rs")).unwrap_err();
    assert!(matches!(err, Error::Forbidden(_)), "got {err:?}");
}
