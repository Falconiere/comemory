//! Embedded React/Vite frontend assets for `comemory serve`.
//!
//! The built SPA under `web/dist/` is compiled into the binary via
//! `rust-embed`, so the shipped `comemory` needs no Node toolchain and no
//! network at runtime. `index.html` carries a `__COMEMORY_TOKEN__` sentinel
//! (in a `<meta>` tag) that is replaced with the live session token when the
//! page is served — mirroring how the static `graph --format html` viewer
//! inlines `__GRAPH_DATA__`. All other assets (Vite's hashed JS/CSS, icons)
//! are served verbatim.

use rust_embed::RustEmbed;

/// The compiled `web/dist/` tree, embedded at build time.
#[derive(RustEmbed)]
#[folder = "web/dist"]
pub struct WebAssets;

/// Sentinel replaced in `index.html` with the per-session token.
const TOKEN_SENTINEL: &str = "__COMEMORY_TOKEN__";

/// Fetch the embedded `index.html` with the token sentinel replaced by
/// `token`. `None` if the frontend has not been built into `web/dist/`.
pub fn index_html_with_token(token: &str) -> Option<String> {
    let file = WebAssets::get("index.html")?;
    let html = std::str::from_utf8(file.data.as_ref()).ok()?;
    Some(html.replace(TOKEN_SENTINEL, token))
}

/// Look up an embedded asset by its `web/dist`-relative key, returning its
/// raw bytes. `None` for keys with no embedded file.
pub fn asset_bytes(key: &str) -> Option<Vec<u8>> {
    WebAssets::get(key).map(|f| f.data.into_owned())
}

/// Best-effort content type for `path` by extension. Defaults to
/// `application/octet-stream` for unknown extensions.
pub fn mime_for(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("");
    match ext {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "webp" => "image/webp",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "wasm" => "application/wasm",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}
