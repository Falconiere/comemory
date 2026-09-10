#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Tests for [`crate::sync::auth_file`].

use std::fs;

use comemory::config::paths::Paths;
use comemory::sync::AuthFile;

use crate::test_common as common;

fn sample_auth() -> AuthFile {
    AuthFile {
        email: Some("dev@example.com".into()),
        ..common::auth_fixture::org_auth("https://api.comemory.io", "cmk_test_secret", "ws-org")
    }
}

#[test]
fn auth_file_roundtrip() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().expect("ensure_dirs");
    let auth = sample_auth();
    auth.save(&paths).expect("save");

    let loaded = AuthFile::load(&paths).expect("load").expect("present");
    assert_eq!(loaded, auth);
}

#[test]
fn auth_file_missing_returns_none() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    assert!(AuthFile::load(&paths).expect("load").is_none());
}

#[cfg(unix)]
#[test]
fn auth_file_saved_with_private_mode() {
    use std::os::unix::fs::PermissionsExt;

    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    paths.ensure_dirs().expect("ensure_dirs");
    sample_auth().save(&paths).expect("save");

    let mode = fs::metadata(paths.auth_file())
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn load_rejects_a_credential_written_before_org_scoping() {
    // The v1 secret is an unbound device key the platform no longer accepts
    // for sync. Parsing it with defaulted org fields would hand the user a
    // credential that reads fine and fails on every call, so the version gate
    // refuses it up front and names the fix.
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path());
    paths.ensure_dirs().unwrap();
    fs::write(
        paths.auth_file(),
        common::auth_fixture::legacy_v1_json("https://api.comemory.io"),
    )
    .unwrap();

    let err = AuthFile::load(&paths).expect_err("a v1 credential must be refused");
    let msg = err.to_string();
    assert!(
        msg.contains("comemory auth login"),
        "message must name the fix, got: {msg}"
    );
    assert!(
        msg.contains("organization scoping"),
        "message must name the cause, got: {msg}"
    );
}

#[test]
fn load_usable_reports_a_v1_credential_as_absent() {
    // Best-effort callers already no-op when auth.json is missing; a stale
    // credential must not turn every local save into a warning.
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path());
    paths.ensure_dirs().unwrap();
    fs::write(
        paths.auth_file(),
        common::auth_fixture::legacy_v1_json("https://api.comemory.io"),
    )
    .unwrap();

    assert!(AuthFile::load_usable(&paths).unwrap().is_none());
}

#[test]
fn load_usable_still_propagates_a_genuinely_broken_file() {
    // Only the version gate is softened. Corrupt JSON is still an error, or a
    // silent no-sync would look identical to "not logged in".
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path());
    paths.ensure_dirs().unwrap();
    fs::write(paths.auth_file(), "{ not json").unwrap();

    assert!(AuthFile::load_usable(&paths).is_err());
}

#[test]
fn round_trip_keeps_the_org_scope_and_stamps_the_version() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path());
    paths.ensure_dirs().unwrap();
    let auth = sample_auth();
    auth.save(&paths).unwrap();

    let loaded = AuthFile::load(&paths).unwrap().expect("present");
    assert_eq!(loaded, auth);
    assert_eq!(
        loaded.version,
        comemory::sync::auth_file::AUTH_SCHEMA_VERSION
    );
    assert_eq!(loaded.workspace_id, "ws-org");
    assert_eq!(loaded.organization_slug, "acme");
}

#[test]
fn clear_removes_a_stale_allowlist_beside_the_credential() {
    // allowlist.json is dead after org scoping; leaving it behind would keep a
    // cached repo list on disk after the credential it belonged to is gone.
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path());
    paths.ensure_dirs().unwrap();
    sample_auth().save(&paths).unwrap();
    fs::write(paths.allowlist_file(), r#"{"repos":[]}"#).unwrap();

    AuthFile::clear(&paths).unwrap();
    assert!(!paths.auth_file().exists());
    assert!(!paths.allowlist_file().exists());

    // Idempotent: logging out twice is not an error.
    AuthFile::clear(&paths).unwrap();
}
