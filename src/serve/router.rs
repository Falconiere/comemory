//! axum router for the graph viewer.

use std::net::SocketAddr;

use axum::extract::Path;
use axum::http::{header, HeaderValue, StatusCode};
use axum::routing::get;
use axum::Router;
use tokio::net::TcpListener;
use tower_http::compression::CompressionLayer;
use tower_http::trace::TraceLayer;

use crate::prelude::*;
use crate::serve::handlers;
use crate::serve::state::ServerState;

const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'";

/// Build the axum router. Public for tests so they can call
/// `Router::oneshot` directly.
pub fn router(state: ServerState) -> Router {
    Router::new()
        .route("/", get(serve_root))
        .route("/{*path}", get(serve_asset_route))
        .route("/api/seed", get(handlers::seed::handle))
        .route("/api/expand", get(handlers::expand::handle))
        .route("/api/search", get(handlers::search::handle))
        .route("/api/node/{id}", get(handlers::node::handle))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn serve_root() -> axum::response::Response {
    let mut resp = crate::serve::assets::serve_asset("/").await;
    resp.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    resp
}

async fn serve_asset_route(Path(path): Path<String>) -> axum::response::Response {
    if path.starts_with("api/") {
        return axum::response::Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(axum::body::Body::from("not found"))
            .unwrap_or_else(|_| {
                axum::response::Response::new(axum::body::Body::from("not found"))
            });
    }
    crate::serve::assets::serve_asset(&format!("/{path}")).await
}

/// Bind to `addr` and serve until SIGINT.
pub async fn run(state: ServerState, addr: SocketAddr, open_browser: bool) -> Result<()> {
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|e| Error::Other(format!("bind {addr}: {e}")))?;
    let bound = listener
        .local_addr()
        .map_err(|e| Error::Other(format!("local_addr: {e}")))?;
    let url = format!("http://{bound}");

    tracing::info!(%url, "qwick-memory graph viewer listening on {url}; press Ctrl-C to stop");

    if open_browser {
        if let Err(e) = open::that(&url) {
            tracing::warn!(error = %e, "could not auto-open browser");
        }
    }

    let app = router(state);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| Error::Other(format!("serve: {e}")))?;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
