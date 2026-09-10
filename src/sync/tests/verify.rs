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

use crate::test_common as common;
use crate::test_common::sync_platform_server::{
    SyncPlatformServer, SyncPlatformState, empty_manifest_buckets,
};

fn seed_auth(paths: &Paths, api_url: &str, secret: &str) {
    common::auth_fixture::seed_org_auth(
        paths,
        api_url,
        secret,
        common::auth_fixture::FIXTURE_WORKSPACE,
    );
}

#[test]
fn matching_empty_manifests_need_no_repair() {
    let platform = SyncPlatformState {
        buckets: Some(empty_manifest_buckets()),
        ..Default::default()
    };
    let server = SyncPlatformServer::start(platform);
    let secret = server.snapshot().secret;

    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("dirs");
    let mut conn = connection::open(paths.db_path()).expect("db");
    let cfg = Config::defaults();
    seed_auth(&paths, &server.base, &secret);

    let auth = AuthFile::load(&paths).expect("load").expect("auth");
    let report = verify::verify_manifests(&paths, &cfg, &mut conn, &auth).expect("verify");
    assert_eq!(report.differing_buckets, 0);
    assert!(report.bucket_indices.is_empty());
    assert!(!report.repaired);
}

#[test]
fn divergent_manifest_runs_repair_then_matches() {
    let mut once = empty_manifest_buckets();
    // First compare sees a flipped bucket; subsequent compares match empty.
    once[0] = "ff".repeat(32);
    let platform = SyncPlatformState {
        buckets_once: Some(once),
        buckets: Some(empty_manifest_buckets()),
        ..Default::default()
    };
    let server = SyncPlatformServer::start(platform);
    let secret = server.snapshot().secret;

    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("dirs");
    let mut conn = connection::open(paths.db_path()).expect("db");
    let cfg = Config::defaults();
    seed_auth(&paths, &server.base, &secret);

    let auth = AuthFile::load(&paths).expect("load").expect("auth");
    let report = verify::verify_manifests(&paths, &cfg, &mut conn, &auth).expect("verify");
    assert!(report.repaired);
    assert_eq!(report.differing_buckets, 0);
}
