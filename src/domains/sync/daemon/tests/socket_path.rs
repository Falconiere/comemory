#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`crate::domains::sync::daemon::socket_path`]: a short data
//! directory keeps its socket beside the data, a long one moves it to a
//! private 0700 runtime directory, and an unsafe directory or socket is
//! refused rather than used.

use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;

use comemory::domains::sync::daemon::socket_path::{
    MAX_SOCKET_PATH, SOCKET_FILE, check_socket, ensure_private_dir, expected, owner_uid, plan,
    private_dir,
};

/// A short private directory for sockets: tempdir paths on macOS are long.
fn short_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("cm")
        .tempdir_in("/tmp")
        .unwrap()
}

fn long_data_dir(root: &std::path::Path) -> PathBuf {
    let dir = root.join("d".repeat(120));
    std::fs::create_dir(&dir).unwrap();
    std::fs::canonicalize(dir).unwrap()
}

#[test]
fn a_short_data_dir_keeps_its_socket_beside_the_data() {
    let dir = short_dir();
    let canonical = std::fs::canonicalize(dir.path()).unwrap();
    assert_eq!(plan(&canonical).unwrap(), canonical.join(SOCKET_FILE));
    assert_eq!(expected(&canonical).unwrap(), canonical.join(SOCKET_FILE));
}

#[test]
fn a_long_data_dir_uses_the_private_runtime_directory() {
    let root = tempfile::tempdir().unwrap();
    let canonical = long_data_dir(root.path());
    assert!(canonical.join(SOCKET_FILE).as_os_str().len() > MAX_SOCKET_PATH);

    let socket = plan(&canonical).unwrap();
    let uid = owner_uid(&canonical).unwrap();
    assert_eq!(socket.parent().unwrap(), private_dir(uid));
    assert!(socket.as_os_str().len() <= MAX_SOCKET_PATH);
    assert_eq!(expected(&canonical).unwrap(), socket);
    let mode = std::fs::metadata(private_dir(uid))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700);
}

#[test]
fn a_loosened_runtime_directory_is_refused_with_the_fix() {
    let root = short_dir();
    let dir = root.path().join("comemory-loose");
    std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
    let uid = owner_uid(root.path()).unwrap();
    let err = ensure_private_dir(&dir, uid).unwrap_err().to_string();
    assert!(err.contains("mode 0700") && err.contains("777"), "{err}");
}

#[test]
fn a_symlink_standing_in_for_the_runtime_directory_is_refused() {
    let root = short_dir();
    let real = root.path().join("real");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&real)
        .unwrap();
    let link = root.path().join("comemory-link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let uid = owner_uid(root.path()).unwrap();
    assert!(ensure_private_dir(&link, uid).is_err());
    ensure_private_dir(&real, uid).unwrap();
}

#[test]
fn only_an_owned_socket_in_a_private_directory_passes() {
    let root = short_dir();
    let uid = owner_uid(root.path()).unwrap();
    let socket = root.path().join("ok.sock");
    let _listener = UnixListener::bind(&socket).unwrap();
    check_socket(&socket, uid).unwrap();

    let err = check_socket(&socket, uid + 1).unwrap_err().to_string();
    assert!(err.contains("belongs to uid"), "{err}");

    let plain = root.path().join("plain.sock");
    std::fs::write(&plain, b"").unwrap();
    assert!(
        check_socket(&plain, uid)
            .unwrap_err()
            .to_string()
            .contains("not a socket")
    );

    let link = root.path().join("link.sock");
    std::os::unix::fs::symlink(&socket, &link).unwrap();
    assert!(
        check_socket(&link, uid)
            .unwrap_err()
            .to_string()
            .contains("not a socket")
    );

    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
    let err = check_socket(&socket, uid).unwrap_err().to_string();
    assert!(err.contains("chmod go-w"), "{err}");
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
}
