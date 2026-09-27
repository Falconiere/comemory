#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`crate::domains::sync::auth_barrier`]: the durable logout
//! barrier hides every credential until an explicit login saves one.

use comemory::config::paths::Paths;
use comemory::domains::sync::AuthFile;
use comemory::domains::sync::auth_barrier;

use crate::test_common as common;

fn logged_in() -> (common::runner::Sandbox, Paths) {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().expect("ensure_dirs");
    common::auth_fixture::org_auth("http://127.0.0.1:9", "cmk_barrier_secret", "ws-org")
        .save(&paths)
        .expect("save");
    (sb, paths)
}

#[test]
fn a_raised_barrier_hides_a_credential_that_is_still_on_disk() {
    let (_sb, paths) = logged_in();
    auth_barrier::raise(&paths).expect("raise");

    assert!(paths.auth_file().exists(), "the file itself is untouched");
    assert!(auth_barrier::active(&paths));
    assert!(AuthFile::load(&paths).expect("load").is_none());
    assert!(AuthFile::load_usable(&paths).expect("load").is_none());
}

#[test]
fn the_status_probe_reports_logged_out_without_dialing() {
    let (_sb, paths) = logged_in();
    auth_barrier::raise(&paths).expect("raise");
    // The credential names a port nothing listens on: a probe that dialed
    // would fail, so `Ok(None)` proves the barrier was checked first.
    let report = comemory::domains::sync::login::status(&paths, None).expect("status");
    assert!(report.is_none());
}

#[test]
fn the_barrier_is_durable_json_naming_the_logout() {
    let (_sb, paths) = logged_in();
    auth_barrier::raise(&paths).expect("raise");
    let raw = std::fs::read_to_string(paths.data_dir().join(auth_barrier::BARRIER_FILE))
        .expect("read barrier");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("json");
    assert_eq!(value["reason"], "logout");
    assert!(value["at"].as_str().is_some_and(|at| at.contains('T')));
}

#[test]
fn saving_a_credential_clears_the_barrier() {
    let (_sb, paths) = logged_in();
    auth_barrier::raise(&paths).expect("raise");
    common::auth_fixture::org_auth("http://127.0.0.1:9", "cmk_next_secret", "ws-org")
        .save(&paths)
        .expect("save");
    assert!(!auth_barrier::active(&paths));
    let loaded = AuthFile::load(&paths).expect("load").expect("present");
    assert_eq!(loaded.secret, "cmk_next_secret");
}

#[test]
fn raising_twice_and_clearing_twice_are_both_idempotent() {
    let (_sb, paths) = logged_in();
    auth_barrier::raise(&paths).expect("raise");
    auth_barrier::raise(&paths).expect("raise again");
    assert!(auth_barrier::active(&paths));
    auth_barrier::clear(&paths).expect("clear");
    auth_barrier::clear(&paths).expect("clear again");
    assert!(!auth_barrier::active(&paths));
}
