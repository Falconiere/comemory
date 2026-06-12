//! Embedded-asset helpers: MIME mapping, token injection, and lookup.

use comemory::serve::assets::{asset_bytes, index_html_with_token, mime_for};

#[test]
fn mime_for_known_and_unknown() {
    assert_eq!(mime_for("app.js"), "text/javascript; charset=utf-8");
    assert_eq!(mime_for("app.css"), "text/css; charset=utf-8");
    assert_eq!(mime_for("index.html"), "text/html; charset=utf-8");
    assert_eq!(mime_for("icon.svg"), "image/svg+xml");
    assert_eq!(mime_for("noext"), "application/octet-stream");
}

#[test]
fn index_html_injects_token_and_drops_sentinel() {
    // Reads the built `web/dist/index.html`; the repo always has a build
    // committed, so this is Some.
    let html = index_html_with_token("deadbeef").expect("web/dist/index.html embedded");
    assert!(html.contains("deadbeef"), "token must be injected");
    assert!(
        !html.contains("__COMEMORY_TOKEN__"),
        "sentinel must be replaced"
    );
}

#[test]
fn asset_bytes_present_for_index_absent_otherwise() {
    assert!(asset_bytes("index.html").is_some());
    assert!(asset_bytes("definitely/missing.bin").is_none());
}
