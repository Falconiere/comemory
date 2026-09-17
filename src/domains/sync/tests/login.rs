#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`crate::domains::sync::login`] — the three `comemory auth`
//! sequences with no rendering attached.
//!
//! The platform is a real loopback HTTP server speaking the real device-code
//! flow, and the credential is a real `auth.json` under a real data dir.

use comemory::config::paths::Paths;
use comemory::domains::sync::{AuthFile, login};

use crate::test_common as common;
use common::device_auth_server::{DeviceAuthServer, tooling_present};

/// `cloud::login` shells out to curl/wget; without either there is no flow to
/// exercise, exactly as the crate-root auth suite decides.
fn http_tools() -> bool {
    tooling_present()
}

#[test]
fn establish_persists_the_minted_credential_and_installs_no_daemon_by_default() {
    if !http_tools() {
        return;
    }
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().expect("ensure_dirs");
    let srv = DeviceAuthServer::start_default();

    let mut progress = Vec::new();
    let established =
        login::establish(&paths, Some(&srv.base), false, &mut progress).expect("establish");

    // The credential the platform minted is on disk, reloadable, and scoped to
    // the organization the mint named.
    let stored = AuthFile::load(&paths).expect("load").expect("present");
    assert_eq!(stored, established.credentials);
    assert_eq!(stored.secret, srv.config.secret);
    assert_eq!(stored.organization_id, srv.config.organization_id);
    assert_eq!(stored.workspace_id, srv.config.workspace_id);
    assert_eq!(stored.api_url, srv.base);

    // Opt-in daemon: nothing was attempted, so there is nothing to report.
    assert!(established.daemon_skipped);
    assert_eq!(established.daemon_running, None);

    // The device-code prompt is the caller's to render, but it must reach the
    // writer it was handed rather than a stream the domain chose.
    assert!(
        !progress.is_empty(),
        "the device prompt never reached the injected writer"
    );

    // The mint route ran; the retired workspace-list route did not.
    assert!(srv.saw_path("/v1/device/mint-org-key"));
    assert!(!srv.saw_path("/v1/workspaces"));
}

#[test]
fn status_reports_logged_out_without_a_credential_or_an_env_override() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().expect("ensure_dirs");

    // No auth.json and no COMEMORY_API_KEY: "logged out" is an outcome the
    // caller renders, not an error the domain raises, and nothing is dialed.
    assert!(
        std::env::var("COMEMORY_API_KEY").is_err(),
        "test environment must not carry a key override"
    );
    assert!(login::status(&paths, None).expect("status").is_none());
}

#[test]
fn logout_removes_the_credential_and_reports_the_daemon_stopped() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().expect("ensure_dirs");
    common::auth_fixture::seed_org_auth(&paths, "https://api.example", "cmk_secret", "ws-org");
    assert!(AuthFile::load(&paths).expect("load").is_some());

    // No daemon was ever installed here, so an unavailable status probe still
    // answers "stopped" — the report must not claim one is running.
    assert!(login::logout(&paths).expect("logout"));
    assert!(AuthFile::load(&paths).expect("load").is_none());

    // Logging out twice is not an error: the file is simply already gone.
    assert!(login::logout(&paths).expect("second logout"));
}
