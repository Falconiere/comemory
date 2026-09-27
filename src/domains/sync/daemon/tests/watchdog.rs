#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! `unbind` removes only the socket inode its own instance bound: a
//! replacement that bound a fresh socket at the same path in the moment
//! after `daemon.lock` was released keeps it (#258). Real Unix sockets on a
//! real filesystem.

use std::os::unix::net::UnixListener;

use super::{inode, unbind};
use crate::config::Paths;

#[test]
fn a_replacements_socket_at_the_same_path_survives_the_old_instances_unbind() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("daemon.sock");
    let old = UnixListener::bind(&socket).unwrap();
    let old_inode = inode(&socket).unwrap();

    // The replacement takes the path over, the way `bind` does.
    std::fs::remove_file(&socket).unwrap();
    let _new = UnixListener::bind(&socket).unwrap();
    let new_inode = inode(&socket).unwrap();
    assert_ne!(old_inode, new_inode);

    unbind(&socket, old_inode, "old", &Paths::new(dir.path()));
    assert_eq!(
        inode(&socket),
        Some(new_inode),
        "the replacement's socket was removed"
    );
    drop(old);
}

#[test]
fn an_instance_removes_its_own_socket() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("daemon.sock");
    let _listener = UnixListener::bind(&socket).unwrap();
    let own = inode(&socket).unwrap();
    unbind(&socket, own, "me", &Paths::new(dir.path()));
    assert!(!socket.exists());
}
