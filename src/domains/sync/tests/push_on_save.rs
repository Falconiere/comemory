#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]

//! The inline push after a local write: it sends, it stays quiet without a
//! credential, and it never lets the network fail a write.

use std::time::Instant;

use comemory::config::{Config, Paths};
use comemory::domains::memories::Kind;
use comemory::domains::memories::save;
use comemory::domains::sync::push_on_save::after_write_best_effort;
use comemory::store::connection;
use comemory::utilities::context::Ctx;

use crate::test_common as common;
use crate::test_common::sync_platform_server::{SyncPlatformServer, SyncPlatformState};

fn save_req(body: &str) -> save::Request {
    save::Request {
        body: body.to_string(),
        title: None,
        kind: Kind::Note,
        // No repo label at all, so repository policy must keep it local.
        repo: String::new(),
        tags: Vec::new(),
        author: String::new(),
        quality: 3,
        supersedes: Vec::new(),
        vector: None,
        ref_file: Vec::new(),
        ref_symbol: Vec::new(),
    }
}

/// Save `body` into a fresh data dir and return its paths (connection closed).
fn saved(paths: &Paths, cfg: &Config, body: &str) {
    paths.ensure_dirs().unwrap();
    let mut conn = connection::open(paths.db_path()).unwrap();
    let mut ctx = Ctx::borrowed(paths, cfg, &mut conn);
    save::run(&mut ctx, save_req(body), false, None).unwrap();
}

#[test]
fn an_unlabelled_save_is_kept_local() {
    let server = SyncPlatformServer::start(SyncPlatformState::default());
    let secret = server.snapshot().secret;

    let body = "a note saved somewhere that is not a git worktree";
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    saved(&paths, &cfg, body);
    common::auth_fixture::seed_org_auth(
        &paths,
        &server.base,
        &secret,
        common::auth_fixture::FIXTURE_WORKSPACE,
    );

    after_write_best_effort(&paths, &cfg);

    assert!(server.saw_path("/v1/sync/status"));
    assert!(server.snapshot().last_import_body.is_none());
}

#[test]
fn nothing_is_sent_when_the_machine_is_not_logged_in() {
    let server = SyncPlatformServer::start(SyncPlatformState::default());
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    saved(&paths, &cfg, "a note on a machine that never logged in");

    after_write_best_effort(&paths, &cfg);

    assert!(
        server.requests().is_empty(),
        "no credential must mean no request at all, saw: {:?}",
        server.paths()
    );
}

#[test]
fn the_hook_is_silent_when_push_on_save_is_off() {
    let server = SyncPlatformServer::start(SyncPlatformState::default());
    let secret = server.snapshot().secret;
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path());
    let mut cfg = Config::defaults();
    cfg.sync.push_on_save = false;
    saved(&paths, &cfg, "a note on a machine that opted out");
    common::auth_fixture::seed_org_auth(
        &paths,
        &server.base,
        &secret,
        common::auth_fixture::FIXTURE_WORKSPACE,
    );

    after_write_best_effort(&paths, &cfg);

    assert!(
        server.requests().is_empty(),
        "push_on_save = false must not reach the network, saw: {:?}",
        server.paths()
    );
}

#[test]
fn an_unreachable_platform_returns_inside_the_budget_and_keeps_the_outbox() {
    // Port 9 (discard) refuses immediately on loopback; the assertion that
    // matters is that the hook returns at all and writes nothing away.
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path());
    let mut cfg = Config::defaults();
    cfg.sync.push_on_save_timeout = "1s".into();
    let body = "a note saved while the platform was unreachable";
    saved(&paths, &cfg, body);
    common::auth_fixture::seed_org_auth(
        &paths,
        "http://127.0.0.1:9",
        "cmk_test",
        common::auth_fixture::FIXTURE_WORKSPACE,
    );

    let started = Instant::now();
    after_write_best_effort(&paths, &cfg);
    let elapsed = started.elapsed();
    assert!(
        elapsed.as_secs() < 5,
        "the hook must not outlive its budget by much, took {elapsed:?}"
    );

    // The entry is still in the outbox: the cursor never advanced past it.
    let conn = connection::open(paths.db_path()).unwrap();
    let state = comemory::store::sync_state::get(&conn, common::auth_fixture::FIXTURE_WORKSPACE)
        .expect("state");
    let pushed_seq = state.map_or(0, |row| row.pushed_seq);
    assert_eq!(
        pushed_seq, 0,
        "a failed push must leave the cursor where it was"
    );
    assert!(
        comemory::store::sync_log::head_seq(&conn).unwrap() > 0,
        "the write is still journalled for the next push"
    );
}

#[test]
fn skips_when_pass_holds_lock() {
    use comemory::domains::sync::auto::hold_pass_lock;

    let server = SyncPlatformServer::start(SyncPlatformState::default());
    let secret = server.snapshot().secret;
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::new(home.path());
    let cfg = Config::defaults();
    saved(&paths, &cfg, "a note saved while a sync pass is running");
    common::auth_fixture::seed_org_auth(
        &paths,
        &server.base,
        &secret,
        common::auth_fixture::FIXTURE_WORKSPACE,
    );
    let pass = hold_pass_lock(&paths).expect("a running pass holds the lock");

    let started = Instant::now();
    after_write_best_effort(&paths, &cfg);

    assert!(
        started.elapsed().as_secs() < 1,
        "the save does not wait for the pass: {:?}",
        started.elapsed()
    );
    assert!(
        server.requests().is_empty(),
        "the running pass sends the write; the inline push made no request: {:?}",
        server.paths()
    );
    drop(pass);
    after_write_best_effort(&paths, &cfg);
    assert!(
        server.saw_path("/v1/sync/status"),
        "with the lock free the inline push runs"
    );
}
