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
        secret: "cmk_test_secret".into(),
        key_prefix: "cmk_test".into(),
        personal_workspace_id: "ws-personal".into(),
        api_url: "https://api.comemory.io".into(),
        device_name: "laptop".into(),
        email: Some("dev@example.com".into()),
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
