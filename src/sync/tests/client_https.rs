#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::too_many_lines
)]

//! HTTPS regression coverage for `sync::client` (#138).
//!
//! The http fixture in `client.rs` never exercises TLS. These tests bind a
//! real rustls listener so a missing reqwest TLS backend cannot regress.
//!
//! The module-level `#![allow(…)]` header is the crate's test-file
//! convention (see CLAUDE.md Testing) — every colocated suite uses it.

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use comemory::sync::client;
use rcgen::{CertifiedKey, generate_simple_self_signed};
use reqwest::blocking::Client;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use serde_json::Value;

/// Same fixture secret shape as `tests/common/sync_platform_server.rs`.
const SECRET: &str = "cmk_test_0000000000000000000000000000000000000000000000000000000000000000";

/// `Error::Other` Display wraps the map_reqwest payload as `other: http: …`.
fn assert_map_reqwest_prefix(msg: &str) {
    assert!(
        msg.starts_with("other: http: "),
        "Error::Other must wrap the map_reqwest `http: ` prefix: {msg}"
    );
}

/// Empty changes envelope matching [`comemory::api::sync::ChangesResponse`].
fn changes_body() -> String {
    r#"{"ok":true,"data":{"entries":[],"next_seq":null,"head_seq":0},"meta":{}}"#.into()
}

/// Loopback rustls server that answers `GET /v1/sync/changes` once per accept.
fn start_https_changes_server(accepts: usize) -> String {
    // Err means a provider is already installed (sibling test in this binary).
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let CertifiedKey { cert, signing_key } =
        generate_simple_self_signed(vec!["localhost".into(), "127.0.0.1".into()])
            .expect("self-signed cert");
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(signing_key.serialize_der()));
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![CertificateDer::from(cert)], key)
        .expect("server config");
    let config = Arc::new(config);

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let remaining = Arc::new(AtomicUsize::new(accepts));
    let body = changes_body();

    thread::spawn(move || {
        while remaining.load(Ordering::SeqCst) > 0 {
            let Ok((stream, _)) = listener.accept() else {
                break;
            };
            remaining.fetch_sub(1, Ordering::SeqCst);
            let conn = ServerConnection::new(Arc::clone(&config)).expect("conn");
            let mut tls = StreamOwned::new(conn, stream);
            let mut buf = [0_u8; 8192];
            let _ = tls.read(&mut buf);
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = tls.write_all(resp.as_bytes());
            let _ = tls.flush();
        }
    });

    format!("https://{addr}")
}

#[test]
fn https_test_client_changes_roundtrip() {
    let base = start_https_changes_server(1);
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .danger_accept_invalid_certs(true)
        .build()
        .expect("client");

    let resp = client
        .get(format!("{base}/v1/sync/changes"))
        .query(&[("since", "0"), ("limit", "50")])
        .header("Authorization", format!("Bearer {SECRET}"))
        .send()
        .expect("https send must complete a TLS handshake");
    assert_eq!(resp.status().as_u16(), 200);
    let env: Value = resp.json().expect("json");
    assert_eq!(env["ok"], true);
    assert_eq!(env["data"]["head_seq"], 0);
    assert!(
        env["data"]["entries"]
            .as_array()
            .expect("entries")
            .is_empty()
    );
}

#[test]
fn https_production_pull_fails_tls_verify() {
    let base = start_https_changes_server(1);
    let err = client::pull_changes(&base, SECRET, 0, 50).expect_err("self-signed must fail verify");
    let msg = err.to_string();
    assert_map_reqwest_prefix(&msg);
    let lower = msg.to_lowercase();
    assert!(
        lower.contains("certificate")
            || lower.contains("cert")
            || lower.contains("tls")
            || lower.contains("handshake")
            || lower.contains("invalid"),
        "must be a TLS/cert failure, not a missing-backend connect drop: {msg}"
    );
}

#[test]
fn connection_refused_error_includes_source() {
    // Bind-and-drop yields a definitely-closed port without racing a listener.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);

    let err = client::pull_changes(&format!("http://127.0.0.1:{port}"), SECRET, 0, 50)
        .expect_err("refused");
    let msg = err.to_string();
    assert_map_reqwest_prefix(&msg);
    // After `other: http: ` comes `<top>: <source>: …`. Connection refused
    // lives in the source chain.
    let after = msg
        .strip_prefix("other: http: ")
        .expect("assert_map_reqwest_prefix already checked");
    assert!(
        after.contains(": "),
        "source chain must appear as nested ': ' segments: {msg}"
    );
    assert!(
        after.to_lowercase().contains("refused") || after.to_lowercase().contains("connect"),
        "refused connect must surface in the chain: {msg}"
    );
}
