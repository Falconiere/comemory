#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]

//! Best-effort auto-sync hooks — no-network early outs + stale detection.

use std::time::Duration;

use time::OffsetDateTime;
use time::format_description::well_known::Iso8601;

use crate::test_common as common;
use comemory::config::{Config, Paths};
use comemory::store::connection;
use comemory::store::sync_state;
use comemory::sync::auto;

#[test]
fn after_save_noop_when_disabled() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("dirs");
    let conn = connection::open(paths.db_path()).expect("db");
    let mut cfg = Config::defaults();
    cfg.sync.after_save = false;
    // Must not panic / spawn a failing push (no auth file), and must hand
    // back nothing to wait on when the knob is off.
    auto::after_save_best_effort(&paths, &cfg, &conn).wait();
}

#[test]
fn after_save_push_is_joinable_so_a_short_lived_process_cannot_lose_it() {
    // `[sync] after_save` runs on its own thread because `reqwest::blocking`
    // cannot run under tokio. A CLI process exits milliseconds after the save
    // returns, so an un-joined handle used to make the push a race it usually
    // lost. Waiting must return once the push finishes — here it fails fast
    // against an unreachable base, which is still a completed push.
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("dirs");
    let conn = connection::open(paths.db_path()).expect("db");
    let cfg = Config::defaults();
    assert!(cfg.sync.after_save, "the shipped default is on");
    common::auth_fixture::seed_org_auth(&paths, "http://127.0.0.1:9", "cmk_test", "ws-org");

    auto::after_save_best_effort(&paths, &cfg, &conn).wait();
}

#[test]
fn pull_before_context_skips_without_auth() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("dirs");
    let mut conn = connection::open(paths.db_path()).expect("db");
    let cfg = Config::defaults();
    auto::pull_before_context_best_effort(&paths, &cfg, &mut conn);
}

#[test]
fn pull_before_context_skips_fresh_last_sync() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("dirs");
    let mut conn = connection::open(paths.db_path()).expect("db");
    let mut cfg = Config::defaults();
    cfg.sync.pull_before_context_after = "1h".into();

    // Unreachable base — must not be contacted when last_sync is fresh.
    let auth = common::auth_fixture::seed_org_auth(
        &paths,
        "http://127.0.0.1:9",
        "cmk_test",
        "ws-personal",
    );

    sync_state::ensure(&conn, "ws-personal", &auth.api_url).expect("ensure");
    let now = OffsetDateTime::now_utc()
        .format(&Iso8601::DEFAULT)
        .expect("iso");
    sync_state::set_pulled(&conn, "ws-personal", 1, &now).expect("stamp");

    // Fresh stamp + 1h threshold → early return before HTTP.
    auto::pull_before_context_best_effort(&paths, &cfg, &mut conn);
    let _ = Duration::from_secs(1);
}
