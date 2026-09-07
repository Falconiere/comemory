#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Unit tests for [`comemory::cloud::credentials`].

use std::fs;

use comemory::cloud::credentials::{Credentials, clear, effective_secret, load, save};
use comemory::config::paths::Paths;

use crate::test_common as common;

#[test]
fn save_load_round_trip_and_mode_0600() {
    let sb = common::runner::Sandbox::new();
    let paths = Paths::new(sb.data_dir());
    let path = paths.auth_file();
    let creds = Credentials {
        api_url: "https://api.example".into(),
        secret: "cmk_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        key_prefix: "cmk_aaaa".into(),
        workspace_id: "ws-1".into(),
    };
    save(&path, &creds).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "auth.json must be mode 0600, got {mode:#o}");
    }
    let loaded = load(&path).unwrap().expect("present");
    assert_eq!(loaded, creds);
    clear(&path).unwrap();
    assert!(load(&path).unwrap().is_none());
}

#[test]
fn comemory_api_key_overrides_file_secret() {
    let prev = std::env::var("COMEMORY_API_KEY").ok();
    let creds = Credentials {
        api_url: "https://api.example".into(),
        secret: "cmk_from_file".into(),
        key_prefix: "cmk_file".into(),
        workspace_id: "ws-1".into(),
    };
    // SAFETY: nextest serializes this suite — set_var/remove_var cannot race.
    unsafe {
        std::env::set_var("COMEMORY_API_KEY", "cmk_from_env");
    }
    assert_eq!(
        effective_secret(Some(&creds)).unwrap().as_deref(),
        Some("cmk_from_env")
    );
    // SAFETY: nextest serializes this suite — set_var/remove_var cannot race.
    unsafe {
        std::env::remove_var("COMEMORY_API_KEY");
    }
    assert_eq!(
        effective_secret(Some(&creds)).unwrap().as_deref(),
        Some("cmk_from_file")
    );
    // SAFETY: restore whatever the process had when the test started.
    unsafe {
        match prev {
            Some(v) => std::env::set_var("COMEMORY_API_KEY", v),
            None => std::env::remove_var("COMEMORY_API_KEY"),
        }
    }
}
