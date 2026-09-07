#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]

//! `verify_manifests` against a loopback platform (match + repair paths).

use comemory::config::{Config, Paths};
use comemory::store::connection;
use comemory::sync::AuthFile;
use comemory::sync::verify;

use crate::test_common::sync_platform_server::{
    SyncPlatformServer, SyncPlatformState, empty_manifest_buckets,
};

fn seed_auth(paths: &Paths, api_url: &str, secret: &str) {
    AuthFile {
        secret: secret.into(),
        key_prefix: "cmk_bbbb".into(),
        personal_workspace_id: "ws-personal".into(),
        api_url: api_url.into(),
        device_name: "test".into(),
        email: None,
    }
    .save(paths)
    .expect("auth");
}

#[test]
fn matching_empty_manifests_need_no_repair() {
    let mut state = SyncPlatformState::default();
    state.buckets = Some(empty_manifest_buckets());
    let server = SyncPlatformServer::start(state);
    let secret = server.snapshot().secret;

    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("dirs");
    let mut conn = connection::open(paths.db_path()).expect("db");
    let cfg = Config::defaults();
    seed_auth(&paths, &server.base, &secret);

    let auth = AuthFile::load(&paths).expect("load").expect("auth");
    let report =
        verify::verify_manifests(&paths, &cfg, &mut conn, &auth, "ws-org").expect("verify");
    assert_eq!(report.differing_buckets, 0);
    assert!(report.bucket_indices.is_empty());
    assert!(!report.repaired);
}

#[test]
fn divergent_manifest_runs_repair_then_matches() {
    let mut once = empty_manifest_buckets();
    // First compare sees a flipped bucket; subsequent compares match empty.
    once[0] = "ff".repeat(32);
    let mut state = SyncPlatformState::default();
    state.buckets_once = Some(once);
    state.buckets = Some(empty_manifest_buckets());
    let server = SyncPlatformServer::start(state);
    let secret = server.snapshot().secret;

    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("dirs");
    let mut conn = connection::open(paths.db_path()).expect("db");
    let cfg = Config::defaults();
    seed_auth(&paths, &server.base, &secret);

    let auth = AuthFile::load(&paths).expect("load").expect("auth");
    let report =
        verify::verify_manifests(&paths, &cfg, &mut conn, &auth, "ws-org").expect("verify");
    assert!(report.repaired);
    assert_eq!(report.differing_buckets, 0);
}
