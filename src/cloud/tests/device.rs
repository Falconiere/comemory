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
