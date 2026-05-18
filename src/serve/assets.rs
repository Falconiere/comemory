//! Embedded frontend assets compiled from `frontend/` via `rust-embed`.

use axum::body::Body;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::Response;
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "frontend/"]
struct Frontend;

/// Resolve `path` to an embedded asset and produce an HTTP response. The
/// empty path and `/` map to `index.html`.
pub async fn serve_asset(path: &str) -> Response {
    let lookup = match path {
        "" | "/" => "index.html".to_string(),
        p => p.trim_start_matches('/').to_string(),
    };
    match Frontend::get(&lookup) {
        Some(file) => {
            let mime = mime_guess::from_path(&lookup)
                .first_or_octet_stream()
                .essence_str()
                .to_string();
            let ct = HeaderValue::from_str(&mime)
                .unwrap_or(HeaderValue::from_static("application/octet-stream"));
            let builder = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, ct);
            match builder.body(Body::from(file.data.into_owned())) {
                Ok(r) => r,
                Err(_) => not_found_response(),
            }
        }
        None => not_found_response(),
    }
}

fn not_found_response() -> Response {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Body::from("not found"))
        .unwrap_or_else(|_| Response::new(Body::from("not found")))
}
