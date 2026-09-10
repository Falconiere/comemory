#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]
//! Unit tests for device-auth JSON shapes and poll-error classification.
//! Full HTTP flow lives in `tests/cli__auth.rs`.

use comemory::cloud::CLIENT_ID;
use comemory::cloud::device::DeviceCodeResponse;

use crate::test_common as common;

#[test]
fn client_id_is_comemory_cli() {
    assert_eq!(CLIENT_ID, "comemory-cli");
}

#[test]
fn device_code_response_deserializes_rfc_fields() {
    let raw = r#"{
        "device_code": "dc-1",
        "user_code": "ABCD-EFGH",
        "verification_uri": "https://api.example/device",
        "verification_uri_complete": "https://api.example/device?user_code=ABCD-EFGH",
        "expires_in": 600,
        "interval": 5
    }"#;
    let parsed: DeviceCodeResponse = serde_json::from_str(raw).unwrap();
    assert_eq!(parsed.device_code, "dc-1");
    assert_eq!(parsed.user_code, "ABCD-EFGH");
    assert_eq!(parsed.expires_in, 600);
    assert_eq!(parsed.interval, Some(5));
    assert_eq!(
        parsed.verification_uri_complete.as_deref(),
        Some("https://api.example/device?user_code=ABCD-EFGH")
    );
}

#[test]
fn poll_error_bodies_carry_rfc_error_codes() {
    for code in [
        "authorization_pending",
        "slow_down",
        "expired_token",
        "access_denied",
        "invalid_grant",
    ] {
        let body = format!(r#"{{"error":"{code}"}}"#);
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["error"], code);
    }
}

#[test]
fn mint_org_key_returns_the_org_scope_over_a_real_socket() {
    // Real HTTP against the loopback platform fixture: `login` shells out
    // through curl/wget, so nothing short of a socket exercises the path.
    if !common::device_auth_server::tooling_present() {
        return;
    }
    let server = common::device_auth_server::DeviceAuthServer::start_default();
    let mut progress = Vec::new();
    let outcome = comemory::cloud::login(&server.base, &mut progress).expect("login");
    let creds = outcome.credentials;

    assert_eq!(
        creds.version,
        comemory::sync::auth_file::AUTH_SCHEMA_VERSION
    );
    assert_eq!(creds.organization_id, server.config.organization_id);
    assert_eq!(creds.organization_slug, "acme");
    assert_eq!(creds.organization_name, "Acme, Inc.");
    assert_eq!(creds.workspace_id, server.config.workspace_id);
    // An empty `apiUrl` from the mint means the deployment set no canonical
    // base, so the dialed URL must survive.
    assert_eq!(creds.api_url, server.base);

    let paths: Vec<String> = server.requests().into_iter().map(|r| r.path).collect();
    assert!(
        paths.contains(&"/v1/device/mint-org-key".to_string()),
        "login must mint the org key, saw: {paths:?}"
    );
    assert!(
        !paths.contains(&"/v1/device/mint-device-key".to_string()),
        "the device-key mint must not be called any more, saw: {paths:?}"
    );

    let printed = String::from_utf8(progress).expect("utf-8 progress");
    assert!(
        printed.contains("USER") || printed.contains("enter code"),
        "the user code must reach the operator, got: {printed}"
    );
}

#[test]
fn unscoped_mint_is_refused_rather_than_persisted() {
    // A key with no organization or workspace can do nothing. Refusing it at
    // the mint keeps a useless credential off disk.
    if !common::device_auth_server::tooling_present() {
        return;
    }
    for config in [
        common::device_auth_server::DeviceAuthConfig {
            workspace_id: String::new(),
            ..Default::default()
        },
        common::device_auth_server::DeviceAuthConfig {
            organization_id: String::new(),
            ..Default::default()
        },
    ] {
        let server = common::device_auth_server::DeviceAuthServer::start(config);
        let mut progress = Vec::new();
        let err = comemory::cloud::login(&server.base, &mut progress)
            .expect_err("an unscoped mint must be refused");
        let msg = err.to_string();
        assert!(
            msg.contains("organization scope"),
            "message must name the cause, got: {msg}"
        );
    }
}

#[test]
fn org_status_reports_a_revoked_key_instead_of_raising() {
    if !common::device_auth_server::tooling_present() {
        return;
    }
    let server = common::device_auth_server::DeviceAuthServer::start_default();
    // The fixture serves no /v1/sync/manifest, so an unknown path answers 404
    // — not an auth rejection, which must stay an error rather than a quiet
    // "not authenticated".
    let err = comemory::cloud::org_status(&server.base, "cmk_wrong", None)
        .expect_err("a non-auth failure must surface");
    assert!(err.to_string().contains("404"), "got: {err}");
}
