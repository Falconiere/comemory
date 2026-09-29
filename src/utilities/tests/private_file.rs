#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Test mirror for `src/utilities/private_file.rs`, on a real temp directory:
//! the file lands whole with mode 0600, replaces an existing one, and a
//! failed write leaves neither the target nor a temp file behind.

use comemory::utilities::private_file::write_atomically;

#[test]
fn writes_whole_owner_only_replaces_and_leaves_nothing_on_failure() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bundle.json");
    std::fs::write(&path, b"old").unwrap();

    write_atomically(&path, b"{\"new\":true}").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"{\"new\":true}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    let missing = dir.path().join("no-such-dir").join("bundle.json");
    assert!(write_atomically(&missing, b"x").is_err());
    assert!(!missing.exists());
    let left: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(left, vec![std::ffi::OsString::from("bundle.json")]);
}
