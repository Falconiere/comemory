//! The security core: token comparison, the loopback Host guard, token
//! generation, and the canonicalize-and-contain path check. A bug in
//! `resolve_within` is a write-anywhere vulnerability, so traversal, absolute,
//! NUL, and symlink-escape cases are all asserted.

use comemory::errors::Error;
use comemory::serve::security::{generate_token, host_is_loopback, resolve_within, token_matches};
use tempfile::TempDir;

#[test]
fn token_matches_only_on_exact() {
    assert!(token_matches(Some("abc"), "abc"));
    assert!(!token_matches(Some("abc"), "abcd"));
    assert!(!token_matches(Some(""), "abc"));
    assert!(!token_matches(None, "abc"));
}

#[test]
fn host_loopback_accepts_only_loopback() {
    for ok in [
        "127.0.0.1",
        "127.0.0.1:8799",
        "localhost",
        "localhost:3000",
        "::1",
        "[::1]:8799",
    ] {
        assert!(host_is_loopback(ok), "{ok} should be loopback");
    }
    for bad in [
        "",
        "evil.com",
        "evil.com:80",
        "127.0.0.1.evil.com",
        "10.0.0.1",
    ] {
        assert!(!host_is_loopback(bad), "{bad} should be rejected");
    }
}

#[test]
fn generate_token_is_64_hex_and_unique() {
    let a = generate_token().expect("token a");
    let b = generate_token().expect("token b");
    assert_eq!(a.len(), 64, "256 bits → 64 hex chars");
    assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(a, b, "tokens must not repeat");
}

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
