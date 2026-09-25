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

/// `domains::sync::cloud::login` shells out to curl/wget; without either there is no flow to
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

    // The device-code prompt must reach the writer it was handed rather than a
    // stream the domain chose — and it must be the prompt, not just any bytes.
    let prompt = String::from_utf8(progress).expect("utf-8 prompt");
    assert!(
        prompt.contains("Visit http://fixture.local/device")
            && prompt.contains("and enter code: USER-"),
        "the injected writer received {prompt:?}"
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
fn logout_stamps_pending_rows_with_the_outgoing_key() {
    use comemory::config::Config;
    use comemory::domains::memories::{Kind, save};
    use comemory::store::connection;
    use comemory::store::replica_outbox::{self, Scope};
    use comemory::utilities::context::Ctx;

    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("dirs");
    let api_url = "http://127.0.0.1:9/api";
    common::auth_fixture::seed_org_auth(&paths, api_url, "cmk_outgoing", "ws_leaving");
    let mut cfg = Config::defaults();
    cfg.sync.after_save = false;
    let mut conn = connection::open(paths.db_path()).expect("db");
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);
    let request = save::Request {
        body: "made while logged in to the workspace being left".to_string(),
        title: None,
        kind: Kind::Note,
        repo: "falconiere/comemory".into(),
        tags: Vec::new(),
        author: String::new(),
        quality: 3,
        supersedes: Vec::new(),
        vector: None,
        ref_file: Vec::new(),
        ref_symbol: Vec::new(),
    };
    save::run(&mut ctx, request, false, None).expect("save");
    drop(ctx);

    login::forget(&paths).expect("forget");

    let rows = replica_outbox::read(&conn, Scope::All, usize::MAX).expect("outbox");
    assert!(!rows.is_empty(), "the save queued an operation");
    for row in rows {
        assert_eq!(row.api_url.as_deref(), Some(api_url), "{row:?}");
        assert_eq!(row.workspace_id.as_deref(), Some("ws_leaving"), "{row:?}");
    }
    assert!(
        AuthFile::load(&paths).expect("load").is_none(),
        "the credential is gone"
    );
}

#[test]
fn logout_of_a_machine_with_no_store_creates_none() {
    let home = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(home.path());
    paths.ensure_dirs().expect("dirs");
    common::auth_fixture::seed_org_auth(&paths, "http://127.0.0.1:9/api", "cmk_x", "ws_x");

    login::forget(&paths).expect("forget");

    assert!(
        !paths.db_path().exists(),
        "no store was created to stamp nothing"
    );
    assert!(AuthFile::load(&paths).expect("load").is_none());
}
