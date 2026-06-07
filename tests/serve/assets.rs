use axum::http::header::CONTENT_TYPE;
use comemory::serve::assets::serve_asset;

fn fetch(path: &str) -> (axum::http::StatusCode, String, Vec<u8>) {
    let resp = futures::executor::block_on(serve_asset(path));
    let status = resp.status();
    let ct = resp
        .headers()
        .get(CONTENT_TYPE)
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    let body = futures::executor::block_on(async {
        axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap()
    });
    (status, ct, body.to_vec())
}

#[test]
fn root_serves_index_html() {
    let (status, ct, body) = fetch("/");
    assert_eq!(status, axum::http::StatusCode::OK);
    assert!(ct.starts_with("text/html"), "ct = {ct}");
    assert!(!body.is_empty());
}

#[test]
fn styles_served() {
    let (status, ct, body) = fetch("/styles.css");
    assert_eq!(status, axum::http::StatusCode::OK);
    assert!(ct.starts_with("text/css"));
    assert!(!body.is_empty());
}

#[test]
fn app_js_served() {
    let (status, ct, body) = fetch("/app.js");
    assert_eq!(status, axum::http::StatusCode::OK);
    assert!(ct.contains("javascript"));
    assert!(!body.is_empty());
}

#[test]
fn unknown_path_returns_404() {
    let (status, _ct, _body) = fetch("/no-such-file");
    assert_eq!(status, axum::http::StatusCode::NOT_FOUND);
}
