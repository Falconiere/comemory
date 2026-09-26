#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`crate::domains::sync::daemon::handshake`].

use std::os::unix::fs::PermissionsExt as _;

use comemory::config::paths::Paths;
use comemory::domains::sync::daemon::handshake::{
    Side, load, load_or_create, matches, proof, token_path,
};

fn home() -> (tempfile::TempDir, Paths) {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path());
    (dir, paths)
}

#[test]
fn the_token_is_created_once_owner_only_and_reused() {
    let (_dir, paths) = home();
    assert!(load(&paths).unwrap().is_none());
    let token = load_or_create(&paths).unwrap();
    assert_eq!(token.len(), 64);
    assert!(token.bytes().all(|b| b.is_ascii_hexdigit()));
    let mode = std::fs::metadata(token_path(&paths))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    assert_eq!(load_or_create(&paths).unwrap(), token);
    assert_eq!(load(&paths).unwrap().as_deref(), Some(token.as_str()));
}

#[test]
fn racing_creators_agree_on_one_token() {
    let (_dir, paths) = home();
    let tokens: Vec<String> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..8)
            .map(|_| s.spawn(|| load_or_create(&paths).unwrap()))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert!(tokens.windows(2).all(|w| w[0] == w[1]), "{tokens:?}");
}

#[test]
fn a_malformed_token_is_refused_with_the_fix() {
    let (_dir, paths) = home();
    std::fs::write(token_path(&paths), "not-a-token").unwrap();
    let err = load(&paths).unwrap_err();
    assert!(err.to_string().contains("sync daemon repair"), "{err}");
}

#[test]
fn proofs_bind_the_side_the_nonce_and_the_token() {
    let token = "a".repeat(64);
    let other = "b".repeat(64);
    let server = proof(Side::Server, "n1", &token);
    assert_eq!(server.len(), 64);
    assert!(matches(&server, &proof(Side::Server, "n1", &token)));
    assert!(
        !matches(&server, &proof(Side::Client, "n1", &token)),
        "sides differ"
    );
    assert!(
        !matches(&server, &proof(Side::Server, "n2", &token)),
        "nonces differ"
    );
    assert!(
        !matches(&server, &proof(Side::Server, "n1", &other)),
        "tokens differ"
    );
    assert!(!matches(&server, &server[..63]), "lengths differ");
}
