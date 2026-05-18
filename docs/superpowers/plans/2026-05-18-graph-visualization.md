# Graph Visualization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship an interactive, browser-based viewer for the kuzu property graph behind `qwick-memory`, exposed as a new `qwick-memory graph serve` subcommand. Loopback-only, read-only, embedded HTML/JS frontend, Cytoscape.js for the canvas.

**Architecture:** A new `src/serve/` module wraps an axum HTTP server that shares an `Arc<Mutex<Graph>>` with the existing kuzu handle. Five REST endpoints (`/`, `/api/seed`, `/api/expand`, `/api/search`, `/api/node/{id}`) serve a vanilla-JS frontend embedded via `rust-embed`. Live Cypher per request — no in-memory cache.

**Tech Stack:** `axum 0.7`, `tokio 1` (already a dep), `tower-http` (`compression-gzip`, `trace`, `set-header`), `rust-embed`, `open`, `siphasher`, `reqwest` (dev-only). Frontend: vanilla JS + vendored `cytoscape.js` + `cose-bilkent` + `marked` + `DOMPurify`. All sanitised markdown is materialised as a `DocumentFragment` via `DOMPurify.sanitize(..., { RETURN_DOM_FRAGMENT: true })` and appended through safe DOM APIs — the frontend never assigns to `innerHTML`.

**Spec:** `docs/superpowers/specs/2026-05-18-graph-visualization-design.md`.

---

## File Map

### New source files (under `src/`)

| File | Responsibility |
|------|----------------|
| `src/cli/graph_serve.rs` | clap `Args`, dispatcher for `graph serve` (port, host, no-open, bind-public). Resolves `Paths`, opens `Graph`, hands to `serve::run`. |
| `src/serve/mod.rs` | Public re-exports + module docstring. Exposes `run` and `ServerState`. |
| `src/serve/state.rs` | `ServerState { graph: Arc<Mutex<Graph>>, paths: Arc<Paths> }` plus its constructor. |
| `src/serve/router.rs` | `pub async fn run(state, addr, open_browser) -> Result<()>` — builds `axum::Router`, binds, prints URL, optional `open`, awaits Ctrl-C. |
| `src/serve/assets.rs` | `rust_embed::RustEmbed` for `frontend/`, `fn serve_asset(uri) -> Response`. |
| `src/serve/error.rs` | `ApiError { code, status, message }` + `IntoResponse` + `From<crate::Error>`. |
| `src/serve/dto.rs` | `NodeDto`, `EdgeDto`, `GraphPayload`, `SearchResult`, `NodeDetail`, `EdgeRef`. `pub fn edge_id(src, kind, tgt) -> String`. |
| `src/serve/handlers/mod.rs` | `pub mod seed; pub mod expand; pub mod search; pub mod node;`. |
| `src/serve/handlers/seed.rs` | `GET /api/seed?layer=…` → `GraphPayload`. |
| `src/serve/handlers/expand.rs` | `GET /api/expand?id=…&depth=…` → `GraphPayload`. |
| `src/serve/handlers/search.rs` | `GET /api/search?q=…&limit=…` → `{ results: [SearchResult] }`. |
| `src/serve/handlers/node.rs` | `GET /api/node/{ns:id}` → `NodeDetail`. |

### Modified source files

| File | Change |
|------|--------|
| `src/lib.rs` | Add `pub mod serve;`. |
| `src/cli/mod.rs` | Add `pub mod graph_serve;`; add `Graph { #[command(subcommand)] cmd: GraphCmd }` variant + nested enum; dispatch in `run`. |
| `src/graph/query.rs` | Add `pub(crate)` methods used by handlers (seed/expand/search/detail). Stays under 500 lines; split into `graph/query/mod.rs` only if needed. |
| `Cargo.toml` | Add `axum`, `tower`, `tower-http`, `rust-embed`, `open`, `siphasher`, `mime_guess`. Add `[dev-dependencies] reqwest`, `tempfile`, `nix`, `libc` (Unix only — `nix` already gated by target). |

### New frontend files (under repo-root `frontend/`)

| File | Responsibility |
|------|----------------|
| `frontend/index.html` | Three-pane layout, vendor `<script>` tags, sidebar form, canvas div. |
| `frontend/styles.css` | Layout, sidebar, detail panel, node-kind color tokens. |
| `frontend/app.js` | Cytoscape init, fetch helpers, render, filters, search, detail panel, Reset key — all via `createElement`/`textContent`/`appendChild`/`replaceChildren`. No `innerHTML` assignments. |
| `frontend/vendor/cytoscape.min.js` | Vendored 3.x. |
| `frontend/vendor/cytoscape-cose-bilkent.min.js` | Layout extension. |
| `frontend/vendor/cose-base.min.js` | Required by cose-bilkent. |
| `frontend/vendor/layout-base.min.js` | Required by cose-bilkent. |
| `frontend/vendor/marked.min.js` | Markdown renderer. |
| `frontend/vendor/purify.min.js` | DOMPurify. |
| `frontend/vendor/README.md` | Upstream URL + version + SHA-256 per file. |

### New test files (under `tests/`)

| File | Responsibility |
|------|----------------|
| `tests/serve.rs` | Thin shim: `mod assets; mod dto; mod error; mod router; mod handlers;`. |
| `tests/serve/assets.rs` | Each embedded asset has expected content-type + non-empty body. |
| `tests/serve/dto.rs` | Serde round-trip for every DTO; `edge_id` stability across runs. |
| `tests/serve/error.rs` | Each `crate::Error` variant maps to expected `ApiError` status + code. |
| `tests/serve/router.rs` | Every declared route resolves; unknown route → 404. |
| `tests/serve/handlers.rs` | `mod seed; mod expand; mod search; mod node;`. |
| `tests/serve/handlers/seed.rs` | `layer=memory` + `layer=all` payloads; bad layer → 400. |
| `tests/serve/handlers/expand.rs` | `depth=1` 1-hop only; depth clamping; unknown id → 404. |
| `tests/serve/handlers/search.rs` | Substring match; `limit` clamping; oversize `q` → 400. |
| `tests/serve/handlers/node.rs` | `Memory` body + frontmatter; non-memory has neither; unknown id → 404. |
| `tests/cli/graph_serve.rs` | Spawn binary `--port 0 --no-open`, parse URL, curl `/api/seed`, kill. |
| `tests/common/graph_fixture.rs` | Build the canonical 3-memory + 2-repo + 1-tag + 1-supersedes + 1-conflict + 1-file + 1-symbol fixture. |

### Modified test files

| File | Change |
|------|--------|
| `tests/cli.rs` | Add `mod graph_serve;` (create if missing). |
| `tests/common.rs` | Add `pub mod graph_fixture;` (create if missing). |

### Docs

| File | Change |
|------|--------|
| `docs/cli-reference.md` | New section for `qwick-memory graph serve` flags + manual smoke checklist. |
| `docs/architecture.md` | Add `serve/` module + embedded frontend paragraph. |
| `README.md` | One-paragraph "Graph viewer" entry under Features. |
| `frontend/vendor/README.md` | Versions + SHA-256 per vendored file. |

---

## Task Decomposition Overview

| # | Task | Phase |
|---|------|-------|
| 1 | Add dependencies to `Cargo.toml`; verify build | Setup |
| 2 | Skeleton `src/serve/` module + register in `lib.rs` | Setup |
| 3 | Edge-id helper (`siphash`) + DTO scaffolding | Backend pure |
| 4 | Remaining DTOs + serde round-trip tests | Backend pure |
| 5 | `ApiError` + `IntoResponse` + `From<crate::Error>` | Backend pure |
| 6 | `tests/common/graph_fixture.rs` | Backend pure |
| 7 | `Graph::seed_memory_layer` + test | Graph queries |
| 8 | `Graph::seed_all` + test | Graph queries |
| 9 | `Graph::expand_neighbors` + test | Graph queries |
| 10 | `Graph::search_nodes` + test | Graph queries |
| 11 | `Graph::node_detail` + test | Graph queries |
| 12 | Embedded assets via `rust-embed` + test | Assets |
| 13 | `serve::router::run` skeleton + state wiring + CSP header | Router |
| 14 | Handler `seed` + test | Handlers |
| 15 | Handler `expand` + test | Handlers |
| 16 | Handler `search` + test | Handlers |
| 17 | Handler `node` + test | Handlers |
| 18 | Router-level tests (unknown route → 404) | Router |
| 19 | `qwick-memory graph serve` CLI wiring | CLI |
| 20 | CLI integration test (spawn + curl + kill) | CLI |
| 21 | Frontend `index.html` + `styles.css` + vendor files | Frontend |
| 22 | Frontend `app.js` — seed render | Frontend |
| 23 | Frontend `app.js` — double-click expand | Frontend |
| 24 | Frontend `app.js` — filter checkboxes + layer toggle | Frontend |
| 25 | Frontend `app.js` — search bar | Frontend |
| 26 | Frontend `app.js` — detail panel + sanitised markdown | Frontend |
| 27 | Frontend `app.js` — Reset key | Frontend |
| 28 | Docs: cli-reference, architecture, README, vendor README | Docs |
| 29 | Final gate: `bash scripts/check-all.sh` + `cargo nextest run --all-features` | Gate |

---

## Task 1: Add dependencies to `Cargo.toml`

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Append runtime deps**

Add the following entries inside the existing `[dependencies]` table (preserve alphabetical placement):

```toml
axum = { version = "0.7", default-features = false, features = ["http1", "json", "tokio", "query"] }
mime_guess = "2"
open = "5"
rust-embed = { version = "8", features = ["debug-embed"] }
siphasher = "1"
tower = { version = "0.5", default-features = false, features = ["util"] }
tower-http = { version = "0.6", features = ["compression-gzip", "trace", "set-header"] }
```

- [ ] **Step 2: Add dev-dependencies**

Append (create the section if absent):

```toml
[dev-dependencies]
reqwest = { version = "0.12", default-features = false, features = ["blocking", "rustls-tls", "json"] }
tempfile = "3"

[target.'cfg(unix)'.dev-dependencies]
libc = "0.2"
nix = { version = "0.29", default-features = false, features = ["signal"] }
```

- [ ] **Step 3: Verify build**

Run: `cargo check --all-targets --all-features`
Expected: success, no warnings.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "build(deps): add axum, tower-http, rust-embed for graph viewer"
```

---

## Task 2: Skeleton `src/serve/` module + register in `lib.rs`

**Files:**
- Create: `src/serve/mod.rs`
- Create: `src/serve/state.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Create `src/serve/state.rs`**

```rust
//! Shared HTTP server state. The kuzu [`Connection`] is `!Sync`, so all
//! handlers serialise behind a `Mutex` around the long-lived [`Graph`].

use std::sync::{Arc, Mutex};

use crate::config::paths::Paths;
use crate::graph::Graph;

/// Long-lived state injected into every axum handler.
#[derive(Clone)]
pub struct ServerState {
    /// Shared kuzu graph handle. Cloning is cheap (`Arc`).
    pub graph: Arc<Mutex<Graph>>,
    /// Resolved data-dir layout. Used by handlers that need to load memory
    /// markdown bodies from disk.
    pub paths: Arc<Paths>,
}

impl ServerState {
    /// Build a new state by taking ownership of the [`Graph`] and the
    /// [`Paths`].
    pub fn new(graph: Graph, paths: Paths) -> Self {
        Self {
            graph: Arc::new(Mutex::new(graph)),
            paths: Arc::new(paths),
        }
    }
}
```

- [ ] **Step 2: Create `src/serve/mod.rs`**

```rust
//! Local HTTP viewer for the kuzu property graph.
//!
//! Exposes `qwick-memory graph serve`. Read-only, loopback-only, embedded
//! frontend served from `frontend/` via `rust-embed`.

pub mod state;

pub use state::ServerState;
```

- [ ] **Step 3: Register the module**

Edit `src/lib.rs` to add `pub mod serve;` immediately after `pub mod cli;`:

```rust
pub mod cli;

pub mod serve;
```

- [ ] **Step 4: Verify compile**

Run: `cargo check --all-targets --all-features`
Expected: success.

- [ ] **Step 5: Commit**

```bash
git add src/serve/ src/lib.rs
git commit -m "feat(serve): scaffold serve module with ServerState"
```

---

## Task 3: Edge-id helper + first DTOs (TDD)

**Files:**
- Create: `src/serve/dto.rs`
- Create: `tests/serve.rs`
- Create: `tests/serve/dto.rs`
- Modify: `src/serve/mod.rs`

- [ ] **Step 1: Create the test shim**

`tests/serve.rs`:

```rust
//! Test binary for the `serve` module. Each submodule mirrors a file under
//! `src/serve/`.

mod dto;
```

- [ ] **Step 2: Write the failing test**

`tests/serve/dto.rs`:

```rust
use qwick_memory::serve::dto::edge_id;

#[test]
fn edge_id_is_deterministic() {
    let a = edge_id("m:a1b2c3d4", "InRepo", "r:qwick-backend");
    let b = edge_id("m:a1b2c3d4", "InRepo", "r:qwick-backend");
    assert_eq!(a, b);
    assert!(a.starts_with("e:"));
    assert_eq!(a.len(), 18, "format is e:<16-hex>");
}

#[test]
fn edge_id_changes_with_kind() {
    let a = edge_id("m:a1b2c3d4", "InRepo", "r:qwick-backend");
    let b = edge_id("m:a1b2c3d4", "Tagged", "r:qwick-backend");
    assert_ne!(a, b);
}

#[test]
fn edge_id_changes_with_endpoints() {
    let a = edge_id("m:aaaa", "InRepo", "r:one");
    let b = edge_id("m:bbbb", "InRepo", "r:one");
    assert_ne!(a, b);
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo nextest run --test serve dto::edge_id_is_deterministic`
Expected: FAIL with "module `dto` not found" or "function `edge_id` not found".

- [ ] **Step 4: Implement `src/serve/dto.rs`**

```rust
//! Wire-level types exchanged with the frontend.
//!
//! Every node id is namespaced (`m:`, `r:`, `a:`, `t:`, `f:`, `s:`) so the
//! frontend can route styling and the backend can resolve to the correct
//! kuzu table. Edge ids are opaque, deterministic 16-hex strings produced
//! by [`edge_id`].

use std::hash::Hasher;

use serde::{Deserialize, Serialize};
use siphasher::sip::SipHasher13;

/// One graph node in either direction (request payload or response).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeDto {
    /// Namespaced id, e.g. `m:a1b2c3d4`.
    pub id: String,
    /// Short human label (often the bare id without prefix).
    pub label: String,
    /// Discriminator matching kuzu node table: `Memory`, `Repo`, `Author`,
    /// `Tag`, `File`, `Symbol`.
    pub kind: String,
    /// Free-form per-kind properties.
    #[serde(default)]
    pub props: serde_json::Value,
}

/// One graph edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeDto {
    /// Opaque deterministic id from [`edge_id`].
    pub id: String,
    pub source: String,
    pub target: String,
    /// kuzu relation table name, e.g. `InRepo`, `Supersedes`.
    pub kind: String,
    #[serde(default)]
    pub props: serde_json::Value,
}

/// Response payload shared by `/api/seed` and `/api/expand`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphPayload {
    pub nodes: Vec<NodeDto>,
    pub edges: Vec<EdgeDto>,
}

/// Compute a deterministic, opaque edge id from the triple.
///
/// Uses [`SipHasher13`] with a fixed all-zero key so the output is stable
/// across processes for the same inputs. The format is `e:<16-hex>`.
pub fn edge_id(source: &str, kind: &str, target: &str) -> String {
    let mut h = SipHasher13::new_with_keys(0, 0);
    h.write(source.as_bytes());
    h.write_u8(0);
    h.write(kind.as_bytes());
    h.write_u8(0);
    h.write(target.as_bytes());
    format!("e:{:016x}", h.finish())
}
```

- [ ] **Step 5: Re-export from `src/serve/mod.rs`**

```rust
pub mod dto;
pub mod state;

pub use state::ServerState;
```

- [ ] **Step 6: Run test to verify it passes**

Run: `cargo nextest run --test serve`
Expected: 3 tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/serve/dto.rs src/serve/mod.rs tests/serve.rs tests/serve/dto.rs
git commit -m "feat(serve): add NodeDto/EdgeDto/GraphPayload + deterministic edge_id"
```

---

## Task 4: Remaining DTOs + serde round-trip tests

**Files:**
- Modify: `src/serve/dto.rs`
- Modify: `tests/serve/dto.rs`

- [ ] **Step 1: Append failing round-trip tests**

Append to `tests/serve/dto.rs`:

```rust
use qwick_memory::serve::dto::{
    EdgeDto, EdgeRef, GraphPayload, NodeDetail, NodeDto, SearchResponse, SearchResult,
};
use serde_json::json;

#[test]
fn node_dto_roundtrip() {
    let n = NodeDto {
        id: "m:a1b2c3d4".into(),
        label: "a1b2c3d4".into(),
        kind: "Memory".into(),
        props: json!({ "quality": 4, "created": "2026-05-17T14:30:00Z" }),
    };
    let s = serde_json::to_string(&n).unwrap();
    let back: NodeDto = serde_json::from_str(&s).unwrap();
    assert_eq!(n, back);
}

#[test]
fn graph_payload_roundtrip() {
    let p = GraphPayload {
        nodes: vec![NodeDto {
            id: "r:one".into(),
            label: "one".into(),
            kind: "Repo".into(),
            props: json!({}),
        }],
        edges: vec![EdgeDto {
            id: "e:0123456789abcdef".into(),
            source: "m:aaaa".into(),
            target: "r:one".into(),
            kind: "InRepo".into(),
            props: json!({}),
        }],
    };
    let s = serde_json::to_string(&p).unwrap();
    let back: GraphPayload = serde_json::from_str(&s).unwrap();
    assert_eq!(p, back);
}

#[test]
fn search_response_roundtrip() {
    let r = SearchResponse {
        results: vec![SearchResult {
            id: "m:aaaa".into(),
            label: "aaaa".into(),
            kind: "Memory".into(),
        }],
    };
    let s = serde_json::to_string(&r).unwrap();
    let back: SearchResponse = serde_json::from_str(&s).unwrap();
    assert_eq!(r, back);
}

#[test]
fn node_detail_roundtrip_memory() {
    let d = NodeDetail {
        node: NodeDto {
            id: "m:aaaa".into(),
            label: "aaaa".into(),
            kind: "Memory".into(),
            props: json!({ "quality": 3 }),
        },
        memory_body: Some("# body".into()),
        frontmatter: Some(json!({ "id": "aaaa" })),
        outbound: vec![EdgeRef {
            edge_kind: "InRepo".into(),
            target: Some("r:one".into()),
            source: None,
        }],
        inbound: vec![EdgeRef {
            edge_kind: "ReferencesFile".into(),
            source: Some("m:bbbb".into()),
            target: None,
        }],
    };
    let s = serde_json::to_string(&d).unwrap();
    let back: NodeDetail = serde_json::from_str(&s).unwrap();
    assert_eq!(d, back);
}

#[test]
fn node_detail_omits_body_for_non_memory() {
    let d = NodeDetail {
        node: NodeDto {
            id: "r:one".into(),
            label: "one".into(),
            kind: "Repo".into(),
            props: json!({}),
        },
        memory_body: None,
        frontmatter: None,
        outbound: vec![],
        inbound: vec![],
    };
    let s = serde_json::to_string(&d).unwrap();
    assert!(!s.contains("memory_body"));
    assert!(!s.contains("frontmatter"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run --test serve dto::`
Expected: 5 new tests fail with "type not found".

- [ ] **Step 3: Extend `src/serve/dto.rs`**

Append to the file:

```rust
/// One row in the search result list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResult {
    pub id: String,
    pub label: String,
    pub kind: String,
}

/// Wrapper for `/api/search` so the frontend reads a stable object shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
}

/// Adjacent edge, used inside [`NodeDetail`]. Exactly one of `target` or
/// `source` is set per direction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeRef {
    pub edge_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// Response payload for `/api/node/{id}`. `memory_body` and `frontmatter`
/// are only set when `node.kind == "Memory"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeDetail {
    pub node: NodeDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frontmatter: Option<serde_json::Value>,
    pub outbound: Vec<EdgeRef>,
    pub inbound: Vec<EdgeRef>,
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run --test serve dto::`
Expected: 8 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/serve/dto.rs tests/serve/dto.rs
git commit -m "feat(serve): add SearchResult/NodeDetail/EdgeRef DTOs"
```

---

## Task 5: `ApiError` + `IntoResponse` + `From<crate::Error>`

**Files:**
- Create: `src/serve/error.rs`
- Create: `tests/serve/error.rs`
- Modify: `src/serve/mod.rs`
- Modify: `tests/serve.rs`

- [ ] **Step 1: Write the failing test**

Create `tests/serve/error.rs`:

```rust
use axum::http::StatusCode;
use axum::response::IntoResponse;
use qwick_memory::errors::Error;
use qwick_memory::serve::error::ApiError;

fn body_string(resp: axum::response::Response) -> (StatusCode, String) {
    let status = resp.status();
    let bytes = futures::executor::block_on(async {
        axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap()
    });
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

#[test]
fn not_found_renders_404_envelope() {
    let e = ApiError::not_found("node m:zzzz");
    let (status, body) = body_string(e.into_response());
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.contains("\"code\":\"not_found\""));
    assert!(body.contains("node m:zzzz"));
}

#[test]
fn invalid_param_renders_400() {
    let (status, body) = body_string(ApiError::invalid_param("bad layer").into_response());
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("\"code\":\"invalid_param\""));
}

#[test]
fn graph_error_maps_to_500() {
    let e: ApiError = Error::Other("kuzu boom".into()).into();
    let (status, body) = body_string(e.into_response());
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(body.contains("\"code\":\"graph_error\""));
}

#[test]
fn io_error_maps_to_500_io_code() {
    let ioe = std::io::Error::new(std::io::ErrorKind::NotFound, "no file");
    let e: ApiError = Error::Io(ioe).into();
    let (status, body) = body_string(e.into_response());
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(body.contains("\"code\":\"io_error\""));
}
```

Update `tests/serve.rs`:

```rust
mod dto;
mod error;
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run --test serve error::`
Expected: FAIL with unresolved imports.

- [ ] **Step 3: Implement `src/serve/error.rs`**

```rust
//! Uniform HTTP error envelope for `/api/...` handlers.
//!
//! Every handler returns `Result<Json<T>, ApiError>`. The wire shape is
//! `{ "error": { "code": "...", "message": "..." } }` and the HTTP status
//! is carried on the response itself.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::errors::Error;

#[derive(Debug, Serialize)]
struct Envelope<'a> {
    error: Inner<'a>,
}

#[derive(Debug, Serialize)]
struct Inner<'a> {
    code: &'a str,
    message: &'a str,
}

/// Error type returned by every handler.
#[derive(Debug)]
pub struct ApiError {
    code: &'static str,
    status: StatusCode,
    message: String,
}

impl ApiError {
    /// 404 — the requested node id is not in the graph.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            code: "not_found",
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    /// 400 — bad query string or body shape.
    pub fn invalid_param(message: impl Into<String>) -> Self {
        Self {
            code: "invalid_param",
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    /// 500 — kuzu query failed.
    pub fn graph_error(message: impl Into<String>) -> Self {
        Self {
            code: "graph_error",
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }

    /// 500 — filesystem failed while loading a memory body.
    pub fn io_error(message: impl Into<String>) -> Self {
        Self {
            code: "io_error",
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        tracing::error!(target: "qwick_memory::serve", code = %self.code, message = %self.message);
        let body = Json(Envelope {
            error: Inner {
                code: self.code,
                message: &self.message,
            },
        });
        (self.status, body).into_response()
    }
}

impl From<Error> for ApiError {
    fn from(e: Error) -> Self {
        match e {
            Error::Io(io) => Self::io_error(io.to_string()),
            Error::Yaml(y) => Self::graph_error(format!("yaml: {y}")),
            Error::Json(j) => Self::graph_error(format!("json: {j}")),
            Error::Toml(t) => Self::graph_error(format!("toml: {t}")),
            Error::Lance(s) => Self::graph_error(format!("lance: {s}")),
            Error::Other(s) => Self::graph_error(s),
        }
    }
}
```

- [ ] **Step 4: Re-export from `src/serve/mod.rs`**

```rust
pub mod dto;
pub mod error;
pub mod state;

pub use error::ApiError;
pub use state::ServerState;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo nextest run --test serve error::`
Expected: 4 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/serve/error.rs src/serve/mod.rs tests/serve.rs tests/serve/error.rs
git commit -m "feat(serve): add ApiError envelope mapping crate::Error to JSON"
```

---

## Task 6: Test fixture builder

**Files:**
- Create: `tests/common/graph_fixture.rs`
- Create / modify: `tests/common.rs`

- [ ] **Step 1: Inspect existing helpers**

Run: `ls tests/common/ 2>/dev/null; rg -n 'pub mod' tests/common.rs 2>/dev/null`

- [ ] **Step 2: Create / update `tests/common.rs`**

If absent:

```rust
//! Shared fixtures for integration tests.

pub mod graph_fixture;
```

Otherwise append `pub mod graph_fixture;`.

- [ ] **Step 3: Identify the existing memory-save entry point**

Run: `rg -n 'pub fn save' src/memory/`

Note the public function name and signature; you will call it from the fixture. If the only public save lives in `src/cli/save.rs` (private to the bin), expose a thin helper from `src/memory/mod.rs`. Add (only if no public save exists already):

```rust
pub fn save_record(paths: &crate::config::paths::Paths, rec: &MemoryRecord) -> Result<()> {
    crate::memory::io::save(paths, rec)
}
```

Adjust the wrapper to the actual existing internal name — do not invent paths.

- [ ] **Step 4: Create `tests/common/graph_fixture.rs`**

```rust
//! Build a deterministic in-memory graph for handler tests.

use std::path::PathBuf;

use qwick_memory::config::paths::Paths;
use qwick_memory::graph::Graph;
use qwick_memory::memory::{Frontmatter, Kind, MemoryRecord};
use tempfile::TempDir;
use time::OffsetDateTime;

/// Owning handle: drop after the test to clean up the temp directory.
pub struct Fixture {
    pub paths: Paths,
    pub graph: Graph,
    _tmp: TempDir,
}

/// Build the canonical fixture and return open handles.
pub fn build() -> Fixture {
    let tmp = tempfile::tempdir().expect("tempdir");
    let paths = Paths::new(PathBuf::from(tmp.path()));
    paths.ensure_dirs().expect("ensure_dirs");
    let graph = Graph::open(paths.graph_dir()).expect("graph open");

    let primary = memory_record("a1b2c3d4", Kind::Decision, &["database", "postgres"], "# primary memory");
    graph.upsert_memory(&primary).expect("upsert primary");

    let old = memory_record("00000001", Kind::Decision, &[], "old");
    graph.upsert_memory(&old).expect("upsert old");

    let conflicting = memory_record("00000002", Kind::Decision, &[], "conflicting");
    graph.upsert_memory(&conflicting).expect("upsert conflicting");

    graph
        .add_supersedes("a1b2c3d4", "00000001")
        .expect("add supersedes");

    {
        let conn = graph.conn().expect("conn");
        conn.query(
            "MATCH (a:Memory {id: 'a1b2c3d4'}), (b:Memory {id: '00000002'}) \
             MERGE (a)-[:ConflictsWith]->(b)",
        )
        .expect("conflicts edge");
    }

    graph
        .upsert_file(
            "qwick-backend:src/db.rs",
            "qwick-backend",
            "src/db.rs",
            &"0".repeat(64),
        )
        .expect("upsert file");
    graph
        .upsert_symbol(
            "qwick-backend:src/db.rs:open",
            "open",
            "fn",
            "rust",
            &"0".repeat(64),
            "qwick-backend:src/db.rs",
        )
        .expect("upsert symbol");
    graph
        .add_references_file("a1b2c3d4", "qwick-backend:src/db.rs")
        .expect("ref file");
    graph
        .add_references_symbol("a1b2c3d4", "qwick-backend:src/db.rs:open")
        .expect("ref symbol");

    // Persist the primary memory's markdown so /api/node/{id} can read it.
    // Substitute the real save fn name discovered in Step 3.
    qwick_memory::memory::save_record(&paths, &primary).expect("save markdown");

    Fixture {
        paths,
        graph,
        _tmp: tmp,
    }
}

fn memory_record(id: &str, kind: Kind, tags: &[&str], body: &str) -> MemoryRecord {
    MemoryRecord {
        frontmatter: Frontmatter {
            id: id.into(),
            kind,
            repo: "qwick-backend".into(),
            tags: tags.iter().map(|s| (*s).to_string()).collect(),
            author: "falconiere".into(),
            created: OffsetDateTime::now_utc(),
            quality: 4,
            schema: 1,
            content_hash: "0".repeat(64),
            references: Default::default(),
            relations: Default::default(),
        },
        body: body.into(),
    }
}
```

If the real `Frontmatter` struct in this repo has different field names or types, adjust the literal — do not invent fields. Run `rg -n 'pub struct Frontmatter' src/memory/` and mirror the existing layout.

- [ ] **Step 5: Smoke-build the fixture**

Add a temporary test to `tests/serve/dto.rs` (DELETE before commit):

```rust
#[test]
fn fixture_builds() {
    #[path = "../common/graph_fixture.rs"]
    mod graph_fixture;
    let _f = graph_fixture::build();
}
```

Run: `cargo nextest run --test serve dto::fixture_builds`
Expected: PASS. Then delete the temporary test.

- [ ] **Step 6: Commit**

```bash
git add tests/common.rs tests/common/graph_fixture.rs src/memory/mod.rs
git commit -m "test: add graph_fixture builder for serve handler tests"
```

---

## Task 7: `Graph::seed_memory_layer`

**Files:**
- Modify: `src/graph/query.rs`
- Modify / create: `tests/graph.rs`
- Modify / create: `tests/graph/query.rs`

- [ ] **Step 1: Inspect existing `tests/graph` layout**

Run: `ls tests/graph/ 2>/dev/null; cat tests/graph.rs 2>/dev/null`

- [ ] **Step 2: Write the failing test**

If `tests/graph.rs` does not exist, create it with `mod query;`. Otherwise add `mod query;`.

In `tests/graph/query.rs` (create if missing) add:

```rust
#[path = "../common/graph_fixture.rs"]
mod graph_fixture;

#[test]
fn seed_memory_layer_returns_memory_repo_author_tag() {
    let fx = graph_fixture::build();
    let payload = fx.graph.seed_memory_layer().expect("seed");
    let kinds: std::collections::BTreeSet<_> =
        payload.nodes.iter().map(|n| n.kind.as_str()).collect();
    assert!(kinds.contains("Memory"));
    assert!(kinds.contains("Repo"));
    assert!(kinds.contains("Author"));
    assert!(kinds.contains("Tag"));
    assert!(!kinds.contains("File"));
    assert!(!kinds.contains("Symbol"));

    let memories = payload.nodes.iter().filter(|n| n.kind == "Memory").count();
    assert_eq!(memories, 3);
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo nextest run --test graph query::seed_memory_layer`
Expected: FAIL with "method not found".

- [ ] **Step 4: Implement `Graph::seed_memory_layer`**

First, verify the kuzu Value variants in your kuzu version. Run: `rg 'pub enum Value' $(cargo metadata --format-version 1 | jq -r '.packages[] | select(.name=="kuzu") | .manifest_path' | xargs dirname)/src` (or open the kuzu docs). Confirm `kuzu::Value::String`, `kuzu::Value::Int64`. Match the existing patterns in `src/graph/query.rs`.

Append to `src/graph/query.rs`:

```rust
use crate::serve::dto::{EdgeDto, GraphPayload, NodeDto, edge_id};
use serde_json::json;

impl Graph {
    /// Memory-layer subgraph: `Memory`, `Repo`, `Author`, `Tag` nodes plus
    /// memory-layer edges.
    pub(crate) fn seed_memory_layer(&self) -> Result<GraphPayload> {
        let conn = self.conn()?;
        let mut nodes = Vec::new();
        let mut edges = Vec::new();

        push_nodes(
            &conn,
            "MATCH (m:Memory) RETURN m.id, m.kind, m.created, m.quality",
            |row| {
                let id = string(&row, 0)?;
                let kind = string(&row, 1)?;
                let created = string(&row, 2)?;
                let quality = int64(&row, 3)?;
                Some(NodeDto {
                    id: format!("m:{id}"),
                    label: id.clone(),
                    kind: "Memory".into(),
                    props: json!({
                        "memory_kind": kind,
                        "created": created,
                        "quality": quality,
                    }),
                })
            },
            &mut nodes,
        )?;
        push_nodes(
            &conn,
            "MATCH (r:Repo) RETURN r.name",
            |row| {
                let n = string(&row, 0)?;
                Some(NodeDto {
                    id: format!("r:{n}"),
                    label: n,
                    kind: "Repo".into(),
                    props: json!({}),
                })
            },
            &mut nodes,
        )?;
        push_nodes(
            &conn,
            "MATCH (a:Author) RETURN a.name",
            |row| {
                let n = string(&row, 0)?;
                Some(NodeDto {
                    id: format!("a:{n}"),
                    label: n,
                    kind: "Author".into(),
                    props: json!({}),
                })
            },
            &mut nodes,
        )?;
        push_nodes(
            &conn,
            "MATCH (t:Tag) RETURN t.name",
            |row| {
                let n = string(&row, 0)?;
                Some(NodeDto {
                    id: format!("t:{n}"),
                    label: n,
                    kind: "Tag".into(),
                    props: json!({}),
                })
            },
            &mut nodes,
        )?;

        push_edges_memory_layer(&conn, &mut edges)?;

        Ok(GraphPayload { nodes, edges })
    }
}

fn push_edges_memory_layer(
    conn: &kuzu::Connection<'_>,
    edges: &mut Vec<EdgeDto>,
) -> Result<()> {
    push_edges(
        conn,
        "MATCH (m:Memory)-[:InRepo]->(r:Repo) RETURN m.id, r.name",
        |a, b| (format!("m:{a}"), "InRepo".to_string(), format!("r:{b}")),
        edges,
    )?;
    push_edges(
        conn,
        "MATCH (m:Memory)-[:AuthoredBy]->(a:Author) RETURN m.id, a.name",
        |a, b| (format!("m:{a}"), "AuthoredBy".to_string(), format!("a:{b}")),
        edges,
    )?;
    push_edges(
        conn,
        "MATCH (m:Memory)-[:Tagged]->(t:Tag) RETURN m.id, t.name",
        |a, b| (format!("m:{a}"), "Tagged".to_string(), format!("t:{b}")),
        edges,
    )?;
    push_edges(
        conn,
        "MATCH (m:Memory)-[:Supersedes]->(n:Memory) RETURN m.id, n.id",
        |a, b| (format!("m:{a}"), "Supersedes".to_string(), format!("m:{b}")),
        edges,
    )?;
    push_edges(
        conn,
        "MATCH (m:Memory)-[:ConflictsWith]->(n:Memory) RETURN m.id, n.id",
        |a, b| (format!("m:{a}"), "ConflictsWith".to_string(), format!("m:{b}")),
        edges,
    )?;
    push_edges(
        conn,
        "MATCH (m:Memory)-[:RelatesTo]->(n:Memory) RETURN m.id, n.id",
        |a, b| (format!("m:{a}"), "RelatesTo".to_string(), format!("m:{b}")),
        edges,
    )?;
    push_edges(
        conn,
        "MATCH (m:Memory)-[:DerivedFrom]->(n:Memory) RETURN m.id, n.id",
        |a, b| (format!("m:{a}"), "DerivedFrom".to_string(), format!("m:{b}")),
        edges,
    )?;
    Ok(())
}

fn push_nodes<F>(
    conn: &kuzu::Connection<'_>,
    cypher: &str,
    mut build: F,
    out: &mut Vec<NodeDto>,
) -> Result<()>
where
    F: FnMut(Vec<kuzu::Value>) -> Option<NodeDto>,
{
    let rs = conn
        .query(cypher)
        .map_err(|e| Error::Other(format!("kuzu query failed: {e}")))?;
    for row in rs {
        if let Some(n) = build(row) {
            out.push(n);
        }
    }
    Ok(())
}

fn push_edges<F>(
    conn: &kuzu::Connection<'_>,
    cypher: &str,
    mut build: F,
    out: &mut Vec<EdgeDto>,
) -> Result<()>
where
    F: FnMut(String, String) -> (String, String, String),
{
    let rs = conn
        .query(cypher)
        .map_err(|e| Error::Other(format!("kuzu query failed: {e}")))?;
    for row in rs {
        let a = match row.first() {
            Some(kuzu::Value::String(s)) => s.clone(),
            _ => continue,
        };
        let b = match row.get(1) {
            Some(kuzu::Value::String(s)) => s.clone(),
            _ => continue,
        };
        let (source, kind, target) = build(a, b);
        out.push(EdgeDto {
            id: edge_id(&source, &kind, &target),
            source,
            target,
            kind,
            props: serde_json::json!({}),
        });
    }
    Ok(())
}

fn string(row: &[kuzu::Value], idx: usize) -> Option<String> {
    match row.get(idx) {
        Some(kuzu::Value::String(s)) => Some(s.clone()),
        _ => None,
    }
}

fn int64(row: &[kuzu::Value], idx: usize) -> Option<i64> {
    match row.get(idx) {
        Some(kuzu::Value::Int64(n)) => Some(*n),
        _ => None,
    }
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo nextest run --test graph query::seed_memory_layer`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/graph/query.rs tests/graph.rs tests/graph/query.rs
git commit -m "feat(graph): add seed_memory_layer + node/edge helpers"
```

---

## Task 8: `Graph::seed_all` (memory + code layers)

**Files:**
- Modify: `src/graph/query.rs`
- Modify: `tests/graph/query.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/graph/query.rs`:

```rust
#[test]
fn seed_all_includes_file_symbol_and_cross_layer_edges() {
    let fx = graph_fixture::build();
    let payload = fx.graph.seed_all().expect("seed all");
    let kinds: std::collections::BTreeSet<_> =
        payload.nodes.iter().map(|n| n.kind.as_str()).collect();
    assert!(kinds.contains("File"));
    assert!(kinds.contains("Symbol"));

    let edge_kinds: std::collections::BTreeSet<_> =
        payload.edges.iter().map(|e| e.kind.as_str()).collect();
    assert!(edge_kinds.contains("ReferencesFile"));
    assert!(edge_kinds.contains("ReferencesSymbol"));
    assert!(edge_kinds.contains("DefinedIn"));
    assert!(edge_kinds.contains("InRepo"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run --test graph query::seed_all`
Expected: FAIL with "method not found".

- [ ] **Step 3: Implement `Graph::seed_all`**

Append to `src/graph/query.rs`:

```rust
impl Graph {
    /// Memory layer plus code layer (`File`, `Symbol`, plus `DefinedIn`,
    /// `Calls`, `Imports`, `ReferencesFile`, `ReferencesSymbol`).
    pub(crate) fn seed_all(&self) -> Result<GraphPayload> {
        let mut payload = self.seed_memory_layer()?;
        let conn = self.conn()?;

        push_nodes(
            &conn,
            "MATCH (f:File) RETURN f.qualified, f.repo, f.path",
            |row| {
                let q = string(&row, 0)?;
                let repo = string(&row, 1)?;
                let path = string(&row, 2)?;
                Some(NodeDto {
                    id: format!("f:{q}"),
                    label: path.clone(),
                    kind: "File".into(),
                    props: json!({ "repo": repo, "path": path }),
                })
            },
            &mut payload.nodes,
        )?;
        push_nodes(
            &conn,
            "MATCH (s:Symbol) RETURN s.qualified, s.name, s.kind, s.language",
            |row| {
                let q = string(&row, 0)?;
                let name = string(&row, 1)?;
                let sk = string(&row, 2)?;
                let lang = string(&row, 3)?;
                Some(NodeDto {
                    id: format!("s:{q}"),
                    label: name.clone(),
                    kind: "Symbol".into(),
                    props: json!({ "name": name, "symbol_kind": sk, "language": lang }),
                })
            },
            &mut payload.nodes,
        )?;

        push_edges(
            &conn,
            "MATCH (s:Symbol)-[:DefinedIn]->(f:File) RETURN s.qualified, f.qualified",
            |a, b| (format!("s:{a}"), "DefinedIn".to_string(), format!("f:{b}")),
            &mut payload.edges,
        )?;
        push_edges(
            &conn,
            "MATCH (s:Symbol)-[:Calls]->(t:Symbol) RETURN s.qualified, t.qualified",
            |a, b| (format!("s:{a}"), "Calls".to_string(), format!("s:{b}")),
            &mut payload.edges,
        )?;
        push_edges(
            &conn,
            "MATCH (a:File)-[:Imports]->(b:File) RETURN a.qualified, b.qualified",
            |a, b| (format!("f:{a}"), "Imports".to_string(), format!("f:{b}")),
            &mut payload.edges,
        )?;
        push_edges(
            &conn,
            "MATCH (m:Memory)-[:ReferencesFile]->(f:File) RETURN m.id, f.qualified",
            |a, b| (format!("m:{a}"), "ReferencesFile".to_string(), format!("f:{b}")),
            &mut payload.edges,
        )?;
        push_edges(
            &conn,
            "MATCH (m:Memory)-[:ReferencesSymbol]->(s:Symbol) RETURN m.id, s.qualified",
            |a, b| (format!("m:{a}"), "ReferencesSymbol".to_string(), format!("s:{b}")),
            &mut payload.edges,
        )?;

        Ok(payload)
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run --test graph query::`
Expected: both tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/graph/query.rs tests/graph/query.rs
git commit -m "feat(graph): add seed_all covering memory + code layers"
```

---

## Task 9: `Graph::expand_neighbors`

**Files:**
- Modify: `src/graph/query.rs`
- Modify: `tests/graph/query.rs`

This task uses a deliberately portable strategy: instead of relying on `kuzu::Value::Node` (whose API varies across kuzu versions), it computes the reachable set of namespaced node ids by walking each adjacent edge query, then filters the precomputed seed-all payload by that set. The result is the same and the implementation reuses helpers from Tasks 7–8.

- [ ] **Step 1: Write the failing test**

Append to `tests/graph/query.rs`:

```rust
#[test]
fn expand_returns_one_hop_for_known_memory() {
    let fx = graph_fixture::build();
    let payload = fx.graph.expand_neighbors("m:a1b2c3d4", 1).expect("expand");
    let ids: std::collections::BTreeSet<_> =
        payload.nodes.iter().map(|n| n.id.clone()).collect();
    assert!(ids.contains("m:a1b2c3d4"));
    assert!(ids.contains("r:qwick-backend"));
    assert!(ids.contains("a:falconiere"));
    assert!(ids.contains("t:database") || ids.contains("t:postgres"));
}

#[test]
fn expand_unknown_id_returns_empty_payload() {
    let fx = graph_fixture::build();
    let payload = fx.graph.expand_neighbors("m:zzzzzzzz", 1).expect("expand");
    assert!(payload.nodes.is_empty());
    assert!(payload.edges.is_empty());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run --test graph query::expand`
Expected: FAIL.

- [ ] **Step 3: Implement `Graph::expand_neighbors`**

Append to `src/graph/query.rs`:

```rust
use std::collections::BTreeSet;

impl Graph {
    /// Return nodes/edges within `depth` hops of `ns_id` (namespaced id).
    /// Depth is clamped to 1..=3 by the HTTP handler.
    pub(crate) fn expand_neighbors(
        &self,
        ns_id: &str,
        depth: u32,
    ) -> Result<GraphPayload> {
        let full = self.seed_all()?;
        if !full.nodes.iter().any(|n| n.id == ns_id) {
            return Ok(GraphPayload::default());
        }

        let mut reachable: BTreeSet<String> = BTreeSet::new();
        reachable.insert(ns_id.to_string());
        for _ in 0..depth.max(1) {
            let mut frontier = BTreeSet::new();
            for e in &full.edges {
                if reachable.contains(&e.source) && !reachable.contains(&e.target) {
                    frontier.insert(e.target.clone());
                } else if reachable.contains(&e.target) && !reachable.contains(&e.source) {
                    frontier.insert(e.source.clone());
                }
            }
            if frontier.is_empty() {
                break;
            }
            reachable.extend(frontier);
        }

        let nodes = full
            .nodes
            .into_iter()
            .filter(|n| reachable.contains(&n.id))
            .collect();
        let edges = full
            .edges
            .into_iter()
            .filter(|e| reachable.contains(&e.source) && reachable.contains(&e.target))
            .collect();

        Ok(GraphPayload { nodes, edges })
    }
}
```

This is O(V·E·depth) on the full graph. For the memory layer (a few hundred nodes) that is sub-ms. If the code layer balloons, swap to a per-id adjacency-pull pass in a follow-up PR.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run --test graph query::expand`
Expected: PASS.

- [ ] **Step 5: Check module size**

Run: `wc -l src/graph/query.rs`
Expected: < 500. If approaching, defer the split until Task 11 and revisit then.

- [ ] **Step 6: Commit**

```bash
git add src/graph/query.rs tests/graph/query.rs
git commit -m "feat(graph): add expand_neighbors via BFS over seed_all"
```

---

## Task 10: `Graph::search_nodes`

**Files:**
- Modify: `src/graph/query.rs`
- Modify: `tests/graph/query.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/graph/query.rs`:

```rust
#[test]
fn search_matches_tag_and_memory_id() {
    let fx = graph_fixture::build();

    let hits = fx.graph.search_nodes("database", 20).expect("search");
    assert!(hits.iter().any(|n| n.kind == "Tag" && n.label == "database"));

    let hits = fx.graph.search_nodes("a1b2", 20).expect("search");
    assert!(hits.iter().any(|n| n.kind == "Memory" && n.id == "m:a1b2c3d4"));
}

#[test]
fn search_respects_limit() {
    let fx = graph_fixture::build();
    let hits = fx.graph.search_nodes("a", 2).expect("search");
    assert!(hits.len() <= 2);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run --test graph query::search`
Expected: FAIL.

- [ ] **Step 3: Implement `Graph::search_nodes`**

Append to `src/graph/query.rs`:

```rust
use crate::serve::dto::SearchResult;

impl Graph {
    /// Case-insensitive substring match across `Memory.id`, `Tag.name`,
    /// `Author.name`, `Repo.name`, `Symbol.name`, `File.path`.
    pub(crate) fn search_nodes(&self, q: &str, limit: usize) -> Result<Vec<SearchResult>> {
        if q.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let needle = q.to_lowercase();
        let conn = self.conn()?;
        let mut hits: Vec<SearchResult> = Vec::new();

        let kinds: [(&str, &str, &str, &str); 6] = [
            ("Memory", "id", "m", "id"),
            ("Tag", "name", "t", "name"),
            ("Author", "name", "a", "name"),
            ("Repo", "name", "r", "name"),
            ("Symbol", "name", "s", "qualified"),
            ("File", "path", "f", "qualified"),
        ];
        for (label, match_field, ns, id_field) in kinds {
            let cypher = format!("MATCH (n:{label}) RETURN n.{match_field}, n.{id_field}");
            let rs = conn
                .query(&cypher)
                .map_err(|e| Error::Other(format!("kuzu query failed: {e}")))?;
            for row in rs {
                let match_val = match row.first() {
                    Some(kuzu::Value::String(s)) => s.clone(),
                    _ => continue,
                };
                let id_val = match row.get(1) {
                    Some(kuzu::Value::String(s)) => s.clone(),
                    _ => continue,
                };
                if !match_val.to_lowercase().contains(&needle) {
                    continue;
                }
                hits.push(SearchResult {
                    id: format!("{ns}:{id_val}"),
                    label: match_val,
                    kind: label.to_string(),
                });
                if hits.len() >= limit {
                    return Ok(hits);
                }
            }
        }
        Ok(hits)
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run --test graph query::search`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/graph/query.rs tests/graph/query.rs
git commit -m "feat(graph): add search_nodes substring matcher across kinds"
```

---

## Task 11: `Graph::node_detail`

**Files:**
- Modify: `src/graph/query.rs`
- Modify: `tests/graph/query.rs`

- [ ] **Step 1: Write the failing test**

Append to `tests/graph/query.rs`:

```rust
#[test]
fn node_detail_for_memory_returns_outbound_edges() {
    let fx = graph_fixture::build();
    let detail = fx.graph.node_detail("m:a1b2c3d4").expect("detail").unwrap();
    assert_eq!(detail.node.kind, "Memory");
    assert_eq!(detail.node.id, "m:a1b2c3d4");
    assert!(detail.outbound.iter().any(|e| e.edge_kind == "InRepo"));
    assert!(detail.outbound.iter().any(|e| e.edge_kind == "Tagged"));
}

#[test]
fn node_detail_unknown_id_returns_none() {
    let fx = graph_fixture::build();
    let detail = fx.graph.node_detail("m:zzzzzzzz").expect("detail");
    assert!(detail.is_none());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run --test graph query::node_detail`
Expected: FAIL.

- [ ] **Step 3: Implement `Graph::node_detail`**

Append to `src/graph/query.rs`:

```rust
use crate::serve::dto::{EdgeRef, NodeDetail};

impl Graph {
    /// Resolve a single namespaced id to its `NodeDetail`. Returns
    /// `Ok(None)` when the node is missing.
    pub(crate) fn node_detail(&self, ns_id: &str) -> Result<Option<NodeDetail>> {
        let full = self.seed_all()?;
        let Some(node) = full.nodes.iter().find(|n| n.id == ns_id).cloned() else {
            return Ok(None);
        };

        let outbound = full
            .edges
            .iter()
            .filter(|e| e.source == ns_id)
            .map(|e| EdgeRef {
                edge_kind: e.kind.clone(),
                target: Some(e.target.clone()),
                source: None,
            })
            .collect();
        let inbound = full
            .edges
            .iter()
            .filter(|e| e.target == ns_id)
            .map(|e| EdgeRef {
                edge_kind: e.kind.clone(),
                target: None,
                source: Some(e.source.clone()),
            })
            .collect();

        Ok(Some(NodeDetail {
            node,
            memory_body: None,
            frontmatter: None,
            outbound,
            inbound,
        }))
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run --test graph query::node_detail`
Expected: PASS.

- [ ] **Step 5: Check module size**

Run: `wc -l src/graph/query.rs`
If `≥ 480`, split now. Create `src/graph/query/` with submodules:

```
src/graph/query/
  mod.rs         # `pub mod common; pub mod seed; pub mod expand; pub mod search; pub mod detail;`
  common.rs      # push_nodes, push_edges, string, int64, push_edges_memory_layer
  seed.rs        # seed_memory_layer + seed_all
  expand.rs      # expand_neighbors
  search.rs      # search_nodes
  detail.rs      # node_detail
```

Move the original `neighbors_by_repo` / `supersedes_chain` / `conflicts_of` into the most natural file (`common.rs` or a new `walk.rs`). Update `tests/graph/query.rs` to remain a single shim — the test file does not need to fan out unless individual tests exceed 500 lines, which they will not.

- [ ] **Step 6: Commit**

```bash
git add src/graph/query.rs tests/graph/query.rs
git commit -m "feat(graph): add node_detail with outbound/inbound edge refs"
```

---

## Task 12: Embedded assets via `rust-embed`

**Files:**
- Create: `src/serve/assets.rs`
- Create: `frontend/index.html` (placeholder)
- Create: `frontend/styles.css` (placeholder)
- Create: `frontend/app.js` (placeholder)
- Create: `frontend/vendor/.gitkeep`
- Create: `tests/serve/assets.rs`
- Modify: `src/serve/mod.rs`
- Modify: `tests/serve.rs`

- [ ] **Step 1: Add placeholder frontend files**

Run: `mkdir -p frontend/vendor && : > frontend/vendor/.gitkeep`

Create `frontend/index.html`:

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <title>qwick-memory graph</title>
    <link rel="stylesheet" href="/styles.css" />
  </head>
  <body>
    <h1>qwick-memory graph viewer</h1>
    <p>placeholder — populated in later task</p>
    <script src="/app.js"></script>
  </body>
</html>
```

Create `frontend/styles.css`:

```css
body { font-family: sans-serif; }
```

Create `frontend/app.js`:

```js
console.log("qwick-memory graph viewer placeholder");
```

- [ ] **Step 2: Write the failing test**

Create `tests/serve/assets.rs`:

```rust
use axum::http::header::CONTENT_TYPE;
use qwick_memory::serve::assets::serve_asset;

fn fetch(path: &str) -> (axum::http::StatusCode, String, Vec<u8>) {
    let resp = futures::executor::block_on(serve_asset(path));
    let status = resp.status();
    let ct = resp
        .headers()
        .get(CONTENT_TYPE)
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default();
    let body = futures::executor::block_on(async {
        axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap()
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
```

Update `tests/serve.rs` to add `mod assets;`.

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo nextest run --test serve assets::`
Expected: FAIL with unresolved import.

- [ ] **Step 4: Implement `src/serve/assets.rs`**

```rust
//! Embedded frontend assets compiled from `frontend/` via `rust-embed`.

use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, header};
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
                Err(_) => Response::new(Body::from("response build error")),
            }
        }
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("not found"))
            .unwrap_or_else(|_| Response::new(Body::from("not found"))),
    }
}
```

- [ ] **Step 5: Re-export from `src/serve/mod.rs`**

```rust
pub mod assets;
pub mod dto;
pub mod error;
pub mod state;

pub use error::ApiError;
pub use state::ServerState;
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo nextest run --test serve assets::`
Expected: 4 tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/serve/assets.rs src/serve/mod.rs frontend/ tests/serve.rs tests/serve/assets.rs
git commit -m "feat(serve): embed frontend assets via rust-embed"
```

---

## Task 13: `serve::router` skeleton + state wiring + CSP header

**Files:**
- Create: `src/serve/router.rs`
- Create: `src/serve/handlers/mod.rs` (+ four handler stubs)
- Modify: `src/serve/mod.rs`

- [ ] **Step 1: Implement `src/serve/router.rs`**

```rust
//! axum router for the graph viewer.

use std::net::SocketAddr;

use axum::Router;
use axum::http::{HeaderValue, header};
use axum::routing::get;
use tokio::net::TcpListener;
use tower_http::compression::CompressionLayer;
use tower_http::trace::TraceLayer;

use crate::prelude::*;
use crate::serve::handlers;
use crate::serve::state::ServerState;

const CSP: &str =
    "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'";

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

async fn serve_asset_route(
    axum::extract::Path(path): axum::extract::Path<String>,
) -> axum::response::Response {
    if path.starts_with("api/") {
        return axum::response::Response::builder()
            .status(axum::http::StatusCode::NOT_FOUND)
            .body(axum::body::Body::from("not found"))
            .unwrap_or_else(|_| axum::response::Response::new(axum::body::Body::from("not found")));
    }
    crate::serve::assets::serve_asset(&format!("/{path}")).await
}

/// Bind to `addr` and serve until SIGINT.
pub async fn run(
    state: ServerState,
    addr: SocketAddr,
    open_browser: bool,
) -> Result<()> {
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|e| Error::Other(format!("bind {addr}: {e}")))?;
    let bound = listener
        .local_addr()
        .map_err(|e| Error::Other(format!("local_addr: {e}")))?;
    let url = format!("http://{bound}");
    tracing::info!(%url, "qwick-memory graph viewer listening");
    println!("qwick-memory graph viewer");
    println!("  listening on {url}");
    println!("  press Ctrl-C to stop");

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
```

- [ ] **Step 2: Add stubbed handlers so the router compiles**

Create `src/serve/handlers/mod.rs`:

```rust
//! HTTP handlers for `/api/*` routes.

pub mod expand;
pub mod node;
pub mod search;
pub mod seed;
```

Create `src/serve/handlers/seed.rs`:

```rust
use axum::Json;
use axum::extract::State;

use crate::serve::dto::GraphPayload;
use crate::serve::error::ApiError;
use crate::serve::state::ServerState;

pub async fn handle(
    State(_): State<ServerState>,
) -> Result<Json<GraphPayload>, ApiError> {
    Err(ApiError::graph_error("not implemented"))
}
```

Create `src/serve/handlers/expand.rs`:

```rust
use axum::Json;
use axum::extract::State;

use crate::serve::dto::GraphPayload;
use crate::serve::error::ApiError;
use crate::serve::state::ServerState;

pub async fn handle(
    State(_): State<ServerState>,
) -> Result<Json<GraphPayload>, ApiError> {
    Err(ApiError::graph_error("not implemented"))
}
```

Create `src/serve/handlers/search.rs`:

```rust
use axum::Json;
use axum::extract::State;

use crate::serve::dto::SearchResponse;
use crate::serve::error::ApiError;
use crate::serve::state::ServerState;

pub async fn handle(
    State(_): State<ServerState>,
) -> Result<Json<SearchResponse>, ApiError> {
    Err(ApiError::graph_error("not implemented"))
}
```

Create `src/serve/handlers/node.rs`:

```rust
use axum::Json;
use axum::extract::State;

use crate::serve::dto::NodeDetail;
use crate::serve::error::ApiError;
use crate::serve::state::ServerState;

pub async fn handle(
    State(_): State<ServerState>,
) -> Result<Json<NodeDetail>, ApiError> {
    Err(ApiError::graph_error("not implemented"))
}
```

- [ ] **Step 3: Update `src/serve/mod.rs`**

```rust
pub mod assets;
pub mod dto;
pub mod error;
pub mod handlers;
pub mod router;
pub mod state;

pub use error::ApiError;
pub use router::run;
pub use state::ServerState;
```

- [ ] **Step 4: Verify compile**

Run: `cargo check --all-targets --all-features`
Expected: success.

- [ ] **Step 5: Commit**

```bash
git add src/serve/
git commit -m "feat(serve): add router::run with CSP header + handler stubs"
```

---

## Task 14: Handler `seed`

**Files:**
- Modify: `src/serve/handlers/seed.rs`
- Create: `tests/serve/handlers.rs`
- Create: `tests/serve/handlers/seed.rs`
- Modify: `tests/serve.rs`

- [ ] **Step 1: Add the handlers test shim**

Create `tests/serve/handlers.rs`:

```rust
//! Per-route handler tests.

#[path = "../common/graph_fixture.rs"]
mod graph_fixture;

mod seed;
```

Append to `tests/serve.rs`:

```rust
mod handlers;
```

- [ ] **Step 2: Write the failing test**

Create `tests/serve/handlers/seed.rs`:

```rust
use axum::body::Body;
use axum::http::{Request, StatusCode};
use qwick_memory::serve::router::router;
use qwick_memory::serve::state::ServerState;
use tower::ServiceExt;

use super::graph_fixture;

async fn get(uri: &str, state: ServerState) -> (StatusCode, serde_json::Value) {
    let app = router(state);
    let resp = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
    (status, body)
}

#[tokio::test]
async fn seed_memory_layer_returns_three_memories() {
    let fx = graph_fixture::build();
    let state = ServerState::new(fx.graph, fx.paths);
    let (status, body) = get("/api/seed?layer=memory", state).await;
    assert_eq!(status, StatusCode::OK);
    let memories = body["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["kind"] == "Memory")
        .count();
    assert_eq!(memories, 3);
    assert!(
        body["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|n| n["kind"] != "File" && n["kind"] != "Symbol"),
        "memory layer should not include File/Symbol"
    );
}

#[tokio::test]
async fn seed_all_includes_code_layer() {
    let fx = graph_fixture::build();
    let state = ServerState::new(fx.graph, fx.paths);
    let (status, body) = get("/api/seed?layer=all", state).await;
    assert_eq!(status, StatusCode::OK);
    let has_file = body["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n["kind"] == "File");
    assert!(has_file);
}

#[tokio::test]
async fn seed_invalid_layer_returns_400() {
    let fx = graph_fixture::build();
    let state = ServerState::new(fx.graph, fx.paths);
    let (status, body) = get("/api/seed?layer=banana", state).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid_param");
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo nextest run --test serve handlers::seed::`
Expected: FAIL (stub returns 500).

- [ ] **Step 4: Implement `src/serve/handlers/seed.rs`**

```rust
//! `GET /api/seed?layer=memory|all` — initial graph payload.

use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;

use crate::serve::dto::GraphPayload;
use crate::serve::error::ApiError;
use crate::serve::state::ServerState;

#[derive(Debug, Deserialize)]
pub struct Params {
    #[serde(default = "default_layer")]
    pub layer: String,
}

fn default_layer() -> String {
    "memory".into()
}

pub async fn handle(
    State(state): State<ServerState>,
    Query(params): Query<Params>,
) -> Result<Json<GraphPayload>, ApiError> {
    let graph = state
        .graph
        .lock()
        .map_err(|_| ApiError::graph_error("graph mutex poisoned"))?;
    let payload = match params.layer.as_str() {
        "memory" => graph.seed_memory_layer()?,
        "all" => graph.seed_all()?,
        other => {
            return Err(ApiError::invalid_param(format!("unknown layer: {other}")));
        }
    };
    Ok(Json(payload))
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo nextest run --test serve handlers::seed::`
Expected: 3 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/serve/handlers/seed.rs tests/serve.rs tests/serve/handlers.rs tests/serve/handlers/seed.rs
git commit -m "feat(serve): implement GET /api/seed"
```

---

## Task 15: Handler `expand`

**Files:**
- Modify: `src/serve/handlers/expand.rs`
- Create: `tests/serve/handlers/expand.rs`
- Modify: `tests/serve/handlers.rs`

- [ ] **Step 1: Write the failing test**

Create `tests/serve/handlers/expand.rs`:

```rust
use axum::body::Body;
use axum::http::{Request, StatusCode};
use qwick_memory::serve::router::router;
use qwick_memory::serve::state::ServerState;
use tower::ServiceExt;

use super::graph_fixture;

async fn get(uri: &str, state: ServerState) -> (StatusCode, serde_json::Value) {
    let app = router(state);
    let resp = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
    (status, body)
}

#[tokio::test]
async fn expand_known_memory_one_hop() {
    let fx = graph_fixture::build();
    let state = ServerState::new(fx.graph, fx.paths);
    let (status, body) = get("/api/expand?id=m:a1b2c3d4&depth=1", state).await;
    assert_eq!(status, StatusCode::OK);
    let nodes = body["nodes"].as_array().unwrap();
    assert!(nodes.iter().any(|n| n["id"] == "m:a1b2c3d4"));
    assert!(nodes.iter().any(|n| n["kind"] == "Repo"));
}

#[tokio::test]
async fn expand_unknown_id_returns_404() {
    let fx = graph_fixture::build();
    let state = ServerState::new(fx.graph, fx.paths);
    let (status, body) = get("/api/expand?id=m:zzzzzzzz&depth=1", state).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");
}

#[tokio::test]
async fn expand_depth_above_three_returns_400() {
    let fx = graph_fixture::build();
    let state = ServerState::new(fx.graph, fx.paths);
    let (status, body) = get("/api/expand?id=m:a1b2c3d4&depth=99", state).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid_param");
}

#[tokio::test]
async fn expand_missing_id_returns_400() {
    let fx = graph_fixture::build();
    let state = ServerState::new(fx.graph, fx.paths);
    let (status, body) = get("/api/expand?depth=1", state).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid_param");
}
```

Append to `tests/serve/handlers.rs`:

```rust
mod expand;
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run --test serve handlers::expand::`
Expected: FAIL.

- [ ] **Step 3: Implement `src/serve/handlers/expand.rs`**

```rust
//! `GET /api/expand?id=<ns:id>&depth=N` — k-hop neighborhood.

use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;

use crate::serve::dto::GraphPayload;
use crate::serve::error::ApiError;
use crate::serve::state::ServerState;

#[derive(Debug, Deserialize)]
pub struct Params {
    pub id: Option<String>,
    #[serde(default = "default_depth")]
    pub depth: u32,
}

fn default_depth() -> u32 {
    1
}

pub async fn handle(
    State(state): State<ServerState>,
    Query(params): Query<Params>,
) -> Result<Json<GraphPayload>, ApiError> {
    let id = params
        .id
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::invalid_param("missing id"))?;
    if params.depth < 1 || params.depth > 3 {
        return Err(ApiError::invalid_param("depth must be 1..=3"));
    }
    let graph = state
        .graph
        .lock()
        .map_err(|_| ApiError::graph_error("graph mutex poisoned"))?;
    let payload = graph.expand_neighbors(&id, params.depth)?;
    if payload.nodes.is_empty() {
        return Err(ApiError::not_found(format!("node {id} not found")));
    }
    Ok(Json(payload))
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run --test serve handlers::expand::`
Expected: 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/serve/handlers/expand.rs tests/serve/handlers.rs tests/serve/handlers/expand.rs
git commit -m "feat(serve): implement GET /api/expand with depth validation"
```

---

## Task 16: Handler `search`

**Files:**
- Modify: `src/serve/handlers/search.rs`
- Create: `tests/serve/handlers/search.rs`
- Modify: `tests/serve/handlers.rs`

- [ ] **Step 1: Write the failing test**

Create `tests/serve/handlers/search.rs`:

```rust
use axum::body::Body;
use axum::http::{Request, StatusCode};
use qwick_memory::serve::router::router;
use qwick_memory::serve::state::ServerState;
use tower::ServiceExt;

use super::graph_fixture;

async fn get(uri: &str, state: ServerState) -> (StatusCode, serde_json::Value) {
    let app = router(state);
    let resp = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
    (status, body)
}

#[tokio::test]
async fn search_matches_tag() {
    let fx = graph_fixture::build();
    let state = ServerState::new(fx.graph, fx.paths);
    let (status, body) = get("/api/search?q=database&limit=20", state).await;
    assert_eq!(status, StatusCode::OK);
    let hit = body["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "Tag" && r["label"] == "database");
    assert!(hit.is_some());
}

#[tokio::test]
async fn search_oversize_q_returns_400() {
    let fx = graph_fixture::build();
    let state = ServerState::new(fx.graph, fx.paths);
    let huge: String = "a".repeat(200);
    let (status, body) = get(&format!("/api/search?q={huge}"), state).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid_param");
}

#[tokio::test]
async fn search_missing_q_returns_400() {
    let fx = graph_fixture::build();
    let state = ServerState::new(fx.graph, fx.paths);
    let (status, body) = get("/api/search", state).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid_param");
}

#[tokio::test]
async fn search_clamps_limit_to_100() {
    let fx = graph_fixture::build();
    let state = ServerState::new(fx.graph, fx.paths);
    let (status, body) = get("/api/search?q=a&limit=9999", state).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["results"].as_array().unwrap().len() <= 100);
}
```

Append to `tests/serve/handlers.rs`:

```rust
mod search;
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run --test serve handlers::search::`
Expected: FAIL.

- [ ] **Step 3: Implement `src/serve/handlers/search.rs`**

```rust
//! `GET /api/search?q=&limit=` — substring match across kinds.

use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;

use crate::serve::dto::SearchResponse;
use crate::serve::error::ApiError;
use crate::serve::state::ServerState;

#[derive(Debug, Deserialize)]
pub struct Params {
    pub q: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

fn default_limit() -> u32 {
    20
}

pub async fn handle(
    State(state): State<ServerState>,
    Query(params): Query<Params>,
) -> Result<Json<SearchResponse>, ApiError> {
    let q = params
        .q
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::invalid_param("missing q"))?;
    if q.len() > 128 {
        return Err(ApiError::invalid_param("q exceeds 128 chars"));
    }
    let limit = params.limit.clamp(1, 100) as usize;
    let graph = state
        .graph
        .lock()
        .map_err(|_| ApiError::graph_error("graph mutex poisoned"))?;
    let results = graph.search_nodes(&q, limit)?;
    Ok(Json(SearchResponse { results }))
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run --test serve handlers::search::`
Expected: 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/serve/handlers/search.rs tests/serve/handlers.rs tests/serve/handlers/search.rs
git commit -m "feat(serve): implement GET /api/search with input validation"
```

---

## Task 17: Handler `node`

**Files:**
- Modify: `src/serve/handlers/node.rs`
- Create: `tests/serve/handlers/node.rs`
- Modify: `tests/serve/handlers.rs`
- Possibly modify: `src/memory/mod.rs` (expose loader)

- [ ] **Step 1: Locate the existing memory loader**

Run: `rg -n 'pub fn load' src/memory/`
Note the public function name, e.g. `pub fn load_by_id(paths: &Paths, id: &str) -> Result<MemoryRecord>`. If only a private loader exists, expose it through `src/memory/mod.rs` with a thin pub wrapper. Do not invent paths.

- [ ] **Step 2: Write the failing test**

Create `tests/serve/handlers/node.rs`:

```rust
use axum::body::Body;
use axum::http::{Request, StatusCode};
use qwick_memory::serve::router::router;
use qwick_memory::serve::state::ServerState;
use tower::ServiceExt;

use super::graph_fixture;

async fn get(uri: &str, state: ServerState) -> (StatusCode, serde_json::Value) {
    let app = router(state);
    let resp = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
    (status, body)
}

#[tokio::test]
async fn node_detail_memory_has_body_and_edges() {
    let fx = graph_fixture::build();
    let state = ServerState::new(fx.graph, fx.paths);
    let (status, body) = get("/api/node/m:a1b2c3d4", state).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["node"]["kind"], "Memory");
    assert!(body["memory_body"].is_string());
    assert!(body["frontmatter"].is_object());
    let outbound_kinds: Vec<String> = body["outbound"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["edge_kind"].as_str().unwrap().to_string())
        .collect();
    assert!(outbound_kinds.iter().any(|k| k == "InRepo"));
}

#[tokio::test]
async fn node_detail_repo_has_no_body() {
    let fx = graph_fixture::build();
    let state = ServerState::new(fx.graph, fx.paths);
    let (status, body) = get("/api/node/r:qwick-backend", state).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["node"]["kind"], "Repo");
    assert!(body.get("memory_body").is_none());
    assert!(body.get("frontmatter").is_none());
}

#[tokio::test]
async fn node_detail_unknown_returns_404() {
    let fx = graph_fixture::build();
    let state = ServerState::new(fx.graph, fx.paths);
    let (status, body) = get("/api/node/m:zzzzzzzz", state).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");
}
```

Append to `tests/serve/handlers.rs`:

```rust
mod node;
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo nextest run --test serve handlers::node::`
Expected: FAIL.

- [ ] **Step 4: Implement `src/serve/handlers/node.rs`**

```rust
//! `GET /api/node/{ns:id}` — full detail for a single node.

use axum::Json;
use axum::extract::{Path, State};

use crate::errors::Error;
use crate::memory;
use crate::serve::dto::NodeDetail;
use crate::serve::error::ApiError;
use crate::serve::state::ServerState;

pub async fn handle(
    State(state): State<ServerState>,
    Path(ns_id): Path<String>,
) -> Result<Json<NodeDetail>, ApiError> {
    let graph = state
        .graph
        .lock()
        .map_err(|_| ApiError::graph_error("graph mutex poisoned"))?;
    let mut detail = graph
        .node_detail(&ns_id)?
        .ok_or_else(|| ApiError::not_found(format!("node {ns_id} not found")))?;

    if detail.node.kind == "Memory" {
        if let Some(raw_id) = ns_id.strip_prefix("m:") {
            // Substitute `load_by_id` with the actual loader function name
            // discovered in Step 1.
            match memory::load_by_id(&state.paths, raw_id) {
                Ok(rec) => {
                    detail.memory_body = Some(rec.body);
                    detail.frontmatter = Some(serde_json::to_value(rec.frontmatter)?);
                }
                Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                    // No markdown body on disk yet — leave fields unset.
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    Ok(Json(detail))
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo nextest run --test serve handlers::node::`
Expected: 3 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/serve/handlers/node.rs tests/serve/handlers.rs tests/serve/handlers/node.rs src/memory/
git commit -m "feat(serve): implement GET /api/node/{id} with markdown body"
```

---

## Task 18: Router-level tests

**Files:**
- Create: `tests/serve/router.rs`
- Modify: `tests/serve.rs`

- [ ] **Step 1: Write the tests**

Create `tests/serve/router.rs`:

```rust
use axum::body::Body;
use axum::http::{Request, StatusCode};
use qwick_memory::serve::router::router;
use qwick_memory::serve::state::ServerState;
use tower::ServiceExt;

#[path = "../common/graph_fixture.rs"]
mod graph_fixture;

#[tokio::test]
async fn unknown_api_route_returns_404() {
    let fx = graph_fixture::build();
    let state = ServerState::new(fx.graph, fx.paths);
    let app = router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/no-such-endpoint")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn unknown_static_path_returns_404() {
    let fx = graph_fixture::build();
    let state = ServerState::new(fx.graph, fx.paths);
    let app = router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/no-such-file")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn root_response_has_csp_header() {
    let fx = graph_fixture::build();
    let state = ServerState::new(fx.graph, fx.paths);
    let app = router(state);
    let resp = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let csp = resp
        .headers()
        .get(axum::http::header::CONTENT_SECURITY_POLICY)
        .expect("csp present")
        .to_str()
        .unwrap();
    assert!(csp.contains("default-src 'self'"));
}
```

Append to `tests/serve.rs`:

```rust
mod router;
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo nextest run --test serve router::`
Expected: 3 tests pass.

- [ ] **Step 3: Commit**

```bash
git add tests/serve.rs tests/serve/router.rs
git commit -m "test(serve): cover router 404 + CSP header"
```

---

## Task 19: `qwick-memory graph serve` CLI wiring

**Files:**
- Create: `src/cli/graph_serve.rs`
- Modify: `src/cli/mod.rs`

- [ ] **Step 1: Implement `src/cli/graph_serve.rs`**

```rust
//! `qwick-memory graph serve` — local HTTP viewer for the property graph.

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::graph::Graph;
use crate::prelude::*;
use crate::serve;

const EXAMPLES: &str = "\
Examples:
  # Open the viewer in the default browser
  qwick-memory graph serve

  # Headless / over SSH
  qwick-memory graph serve --no-open

  # Pin a port
  qwick-memory graph serve --port 7878";

#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Override the bind port. `0` lets the kernel pick a free port.
    #[arg(long, default_value_t = 0)]
    pub port: u16,
    /// Skip auto-opening the URL in the system browser.
    #[arg(long)]
    pub no_open: bool,
    /// Bind address. Loopback by default.
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,
    /// Required when `--host` is non-loopback. Acknowledges the network
    /// exposure: the viewer is read-only but unauthenticated.
    #[arg(long)]
    pub bind_public: bool,
}

pub async fn run(a: Args, _json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let host: IpAddr = a
        .host
        .parse()
        .map_err(|e| Error::Other(format!("--host {host}: {e}", host = a.host)))?;
    if !host.is_loopback() && !a.bind_public {
        return Err(Error::Other(
            "non-loopback --host requires --bind-public".into(),
        ));
    }
    if !host.is_loopback() {
        tracing::warn!(%host, "qwick-memory graph serve is binding to a public address");
    }

    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let graph = Graph::open(paths.graph_dir())?;
    let state = serve::ServerState::new(graph, paths);
    let addr = SocketAddr::new(host, a.port);
    serve::router::run(state, addr, !a.no_open).await
}
```

- [ ] **Step 2: Register the nested subcommand in `src/cli/mod.rs`**

Add the module:

```rust
pub mod graph_serve;
```

Add the parent `Graph` variant inside `enum Cmd`:

```rust
/// Property-graph tooling. Run `qwick-memory graph --help`.
Graph {
    #[command(subcommand)]
    cmd: GraphCmd,
},
```

Add the nested enum below `Cmd`:

```rust
#[derive(Subcommand, Debug)]
pub enum GraphCmd {
    /// Spin up the local HTTP viewer for the property graph.
    Serve(graph_serve::Args),
}
```

Add the dispatcher arm inside `pub async fn run(cli: Cli)`:

```rust
Cmd::Graph { cmd } => match cmd {
    GraphCmd::Serve(a) => graph_serve::run(a, cli.json, cli.data_dir).await,
},
```

- [ ] **Step 3: Verify compile**

Run: `cargo check --all-targets --all-features`
Expected: success.

- [ ] **Step 4: Manual smoke**

In one shell:

```bash
cargo run --quiet -- graph serve --port 0 --no-open
```

Note the printed URL. In another shell:

```bash
curl -s "${URL}/api/seed?layer=memory" | jq '.nodes | length'
```

Expected: a number ≥ 0. Stop the server with Ctrl-C; the process exits cleanly.

- [ ] **Step 5: Commit**

```bash
git add src/cli/graph_serve.rs src/cli/mod.rs
git commit -m "feat(cli): add qwick-memory graph serve subcommand"
```

---

## Task 20: CLI integration test

**Files:**
- Create: `tests/cli/graph_serve.rs`
- Create / modify: `tests/cli.rs`

- [ ] **Step 1: Create or update `tests/cli.rs`**

If absent:

```rust
//! Integration tests against the real `qwick-memory` binary.

mod graph_serve;
```

Otherwise append `mod graph_serve;`.

- [ ] **Step 2: Write the test**

Create `tests/cli/graph_serve.rs`:

```rust
//! Spawn `qwick-memory graph serve --port 0 --no-open`, read the
//! printed URL, hit `/api/seed`, and shut down.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use tempfile::TempDir;

#[test]
fn graph_serve_starts_and_serves_seed() {
    let tmp = TempDir::new().expect("tempdir");

    let bin = env!("CARGO_BIN_EXE_qwick-memory");
    let mut child = Command::new(bin)
        .args(["graph", "serve", "--port", "0", "--no-open"])
        .env("QWICK_MEMORY_DATA_DIR", tmp.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn qwick-memory");

    let stdout = child.stdout.take().expect("child stdout");
    let mut reader = BufReader::new(stdout);

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut url: Option<String> = None;
    while Instant::now() < deadline {
        let mut line = String::new();
        let n = reader.read_line(&mut line).expect("read_line");
        if n == 0 {
            break;
        }
        if let Some(rest) = line.trim().strip_prefix("listening on ") {
            url = Some(rest.to_string());
            break;
        }
    }
    let url = url.expect("did not see listening line");

    let body: serde_json::Value = reqwest::blocking::get(format!("{url}/api/seed?layer=memory"))
        .expect("GET /api/seed")
        .json()
        .expect("parse json");
    assert!(body.get("nodes").is_some());
    assert!(body.get("edges").is_some());

    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(child.id() as i32),
            nix::sys::signal::Signal::SIGINT,
        )
        .expect("sigint");
        let status = child.wait().expect("wait");
        assert!(
            status.success() || status.signal() == Some(libc::SIGINT),
            "unexpected exit: {status:?}"
        );
    }
    #[cfg(not(unix))]
    {
        child.kill().expect("kill");
        let _ = child.wait();
    }
}
```

- [ ] **Step 3: Run the test**

Run: `cargo nextest run --test cli graph_serve`
Expected: PASS within ~5 seconds.

- [ ] **Step 4: Commit**

```bash
git add tests/cli.rs tests/cli/graph_serve.rs
git commit -m "test(cli): cover qwick-memory graph serve startup + shutdown"
```

---

## Task 21: Frontend `index.html` + `styles.css` + vendor files

**Files:**
- Modify: `frontend/index.html`
- Modify: `frontend/styles.css`
- Create: every file under `frontend/vendor/*.js`
- Create: `frontend/vendor/README.md`

- [ ] **Step 1: Download pinned vendor files**

```bash
cd frontend/vendor
curl -L -o cytoscape.min.js                 https://unpkg.com/cytoscape@3.30.2/dist/cytoscape.min.js
curl -L -o cose-base.min.js                 https://unpkg.com/cose-base@2.2.0/cose-base.js
curl -L -o layout-base.min.js               https://unpkg.com/layout-base@2.0.1/layout-base.js
curl -L -o cytoscape-cose-bilkent.min.js    https://unpkg.com/cytoscape-cose-bilkent@4.1.0/cytoscape-cose-bilkent.js
curl -L -o marked.min.js                    https://unpkg.com/marked@14.1.3/marked.min.js
curl -L -o purify.min.js                    https://unpkg.com/dompurify@3.1.7/dist/purify.min.js
shasum -a 256 *.js > checksums.txt
cd -
```

- [ ] **Step 2: Write `frontend/vendor/README.md`**

```markdown
# Vendored frontend dependencies

Pinned versions used by `qwick-memory graph serve`. Bumping any of these
requires re-running the smoke checklist in `docs/cli-reference.md`.

| File | Upstream | Version |
|------|----------|---------|
| cytoscape.min.js | https://unpkg.com/cytoscape@3.30.2/dist/cytoscape.min.js | 3.30.2 |
| cose-base.min.js | https://unpkg.com/cose-base@2.2.0/cose-base.js | 2.2.0 |
| layout-base.min.js | https://unpkg.com/layout-base@2.0.1/layout-base.js | 2.0.1 |
| cytoscape-cose-bilkent.min.js | https://unpkg.com/cytoscape-cose-bilkent@4.1.0/cytoscape-cose-bilkent.js | 4.1.0 |
| marked.min.js | https://unpkg.com/marked@14.1.3/marked.min.js | 14.1.3 |
| purify.min.js | https://unpkg.com/dompurify@3.1.7/dist/purify.min.js | 3.1.7 |

Verify with `shasum -a 256 *.js` against `checksums.txt`.
```

- [ ] **Step 3: Replace `frontend/index.html`**

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <title>qwick-memory graph</title>
    <link rel="stylesheet" href="/styles.css" />
  </head>
  <body>
    <header>
      <h1>qwick-memory graph</h1>
      <input id="q" type="search" placeholder="search nodes…" autocomplete="off" />
      <ul id="results" hidden></ul>
    </header>
    <main>
      <aside id="sidebar">
        <fieldset>
          <legend>Layer</legend>
          <label><input type="checkbox" data-layer="memory" checked /> Memory</label>
          <label><input type="checkbox" data-layer="code" /> Code</label>
        </fieldset>
        <fieldset id="kinds">
          <legend>Kinds</legend>
        </fieldset>
        <button id="reset" type="button">Reset (R)</button>
      </aside>
      <section id="canvas"></section>
      <aside id="detail">
        <p class="muted">Click a node to see details.</p>
      </aside>
    </main>
    <script src="/vendor/cytoscape.min.js"></script>
    <script src="/vendor/layout-base.min.js"></script>
    <script src="/vendor/cose-base.min.js"></script>
    <script src="/vendor/cytoscape-cose-bilkent.min.js"></script>
    <script src="/vendor/marked.min.js"></script>
    <script src="/vendor/purify.min.js"></script>
    <script src="/app.js"></script>
  </body>
</html>
```

- [ ] **Step 4: Replace `frontend/styles.css`**

```css
* { box-sizing: border-box; }
html, body { height: 100%; margin: 0; font-family: system-ui, sans-serif; }
header {
  display: flex; align-items: center; gap: 1em;
  padding: 0.5em 1em; border-bottom: 1px solid #ddd; background: #fafafa;
}
header h1 { font-size: 1rem; margin: 0; }
header input[type="search"] { flex: 1; padding: 0.4em 0.6em; }
#results {
  position: absolute; top: 3rem; right: 1rem; width: 24rem; z-index: 10;
  background: white; border: 1px solid #ccc; max-height: 60vh; overflow: auto;
  list-style: none; margin: 0; padding: 0;
}
#results li { padding: 0.4em 0.6em; cursor: pointer; }
#results li:hover { background: #f0f0f0; }
main { display: grid; grid-template-columns: 14rem 1fr 22rem; height: calc(100vh - 3rem); }
#sidebar, #detail { padding: 1em; overflow: auto; border-right: 1px solid #ddd; }
#detail { border-right: none; border-left: 1px solid #ddd; }
#canvas { background: #fff; }
fieldset { border: 1px solid #ddd; margin-bottom: 1em; }
legend { font-size: 0.85rem; color: #555; }
label { display: block; margin: 0.2em 0; cursor: pointer; }
.muted { color: #888; }
button { padding: 0.4em 0.8em; cursor: pointer; }
.detail-block { margin-bottom: 1em; }
.detail-block h3 { margin: 0 0 0.4em 0; font-size: 0.9rem; color: #444; }
.edge-list { padding-left: 1em; margin: 0; }
```

- [ ] **Step 5: Re-run the asset tests**

Run: `cargo nextest run --test serve assets::`
Expected: 4 tests still pass.

- [ ] **Step 6: Commit**

```bash
git add frontend/
git commit -m "feat(frontend): scaffold layout + vendor cytoscape/marked/dompurify"
```

---

## Task 22: Frontend `app.js` — seed render

**Files:**
- Modify: `frontend/app.js`

Frontend code uses only safe DOM APIs (`createElement`, `textContent`, `appendChild`, `replaceChildren`). Sanitised markdown is materialised through `DOMPurify.sanitize(html, { RETURN_DOM_FRAGMENT: true })` and appended as a `DocumentFragment` — no `innerHTML` assignment anywhere.

- [ ] **Step 1: Replace `frontend/app.js`**

```javascript
// qwick-memory graph viewer. Vanilla JS, no build step, no innerHTML.

const KIND_COLOR = {
  Memory: "#2b6cb0",
  Repo: "#2f855a",
  Author: "#6b46c1",
  Tag: "#718096",
  File: "#dd6b20",
  Symbol: "#c53030",
};

const cy = cytoscape({
  container: document.getElementById("canvas"),
  elements: [],
  style: [
    {
      selector: "node",
      style: {
        "background-color": (ele) => KIND_COLOR[ele.data("kind")] || "#888",
        label: "data(label)",
        color: "#222",
        "font-size": 10,
        "text-wrap": "ellipsis",
        "text-max-width": 80,
      },
    },
    {
      selector: "edge",
      style: {
        width: 1,
        "line-color": "#bbb",
        "target-arrow-color": "#bbb",
        "target-arrow-shape": "triangle",
        "curve-style": "bezier",
        "font-size": 9,
        label: "data(kind)",
        color: "#666",
      },
    },
  ],
  wheelSensitivity: 0.2,
});

async function fetchJson(url) {
  const resp = await fetch(url, { headers: { Accept: "application/json" } });
  if (!resp.ok) {
    const err = await resp.json().catch(() => ({ error: { code: "unknown", message: resp.statusText } }));
    throw new Error(`${resp.status} ${err.error?.code || "error"}: ${err.error?.message || ""}`);
  }
  return resp.json();
}

function toElements(payload) {
  const nodes = payload.nodes.map((n) => ({
    data: { id: n.id, label: n.label, kind: n.kind, props: n.props },
  }));
  const edges = payload.edges.map((e) => ({
    data: {
      id: e.id,
      source: e.source,
      target: e.target,
      kind: e.kind,
      props: e.props,
    },
  }));
  return [...nodes, ...edges];
}

function mergeElements(payload) {
  for (const el of toElements(payload)) {
    if (!cy.getElementById(el.data.id).nonempty()) {
      cy.add(el);
    }
  }
}

function runLayout(animate = false) {
  cy.layout({
    name: "cose-bilkent",
    animate,
    nodeRepulsion: 4500,
    idealEdgeLength: 80,
    gravity: 0.25,
  }).run();
}

async function loadSeed(layer = "memory") {
  cy.elements().remove();
  const payload = await fetchJson(`/api/seed?layer=${encodeURIComponent(layer)}`);
  mergeElements(payload);
  runLayout(false);
}

function showDetailMessage(text) {
  const el = document.getElementById("detail");
  el.replaceChildren();
  const p = document.createElement("p");
  p.className = "muted";
  p.textContent = text;
  el.appendChild(p);
}

window.addEventListener("DOMContentLoaded", () => {
  loadSeed("memory").catch((e) => {
    console.error(e);
    showDetailMessage(`Failed to load graph: ${e.message}`);
  });
});
```

- [ ] **Step 2: Manual smoke**

Run: `cargo run --quiet -- graph serve --port 7878 --no-open`
Open `http://127.0.0.1:7878/`. Verify the memory layer renders with the colors above. Stop with Ctrl-C.
Expected: PASS by eye.

- [ ] **Step 3: Commit**

```bash
git add frontend/app.js
git commit -m "feat(frontend): render seed payload via Cytoscape"
```

---

## Task 23: Frontend `app.js` — double-click expand

**Files:**
- Modify: `frontend/app.js`

- [ ] **Step 1: Append the handler at the bottom of `app.js`**

```javascript
cy.on("dblclick", "node", async (evt) => {
  const node = evt.target;
  try {
    const payload = await fetchJson(
      `/api/expand?id=${encodeURIComponent(node.id())}&depth=1`,
    );
    mergeElements(payload);
    runLayout(false);
  } catch (e) {
    console.error("expand failed", e);
  }
});
```

- [ ] **Step 2: Manual smoke**

Run the viewer; double-click a Memory node; verify neighbours load.
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add frontend/app.js
git commit -m "feat(frontend): double-click node to expand 1-hop"
```

---

## Task 24: Frontend `app.js` — filter checkboxes + layer toggle

**Files:**
- Modify: `frontend/app.js`

- [ ] **Step 1: Append filter wiring**

```javascript
const ALL_KINDS = ["Memory", "Repo", "Author", "Tag", "File", "Symbol"];

function renderKindFilters() {
  const fs = document.getElementById("kinds");
  fs.replaceChildren();
  const legend = document.createElement("legend");
  legend.textContent = "Kinds";
  fs.appendChild(legend);
  for (const k of ALL_KINDS) {
    const label = document.createElement("label");
    const input = document.createElement("input");
    input.type = "checkbox";
    input.dataset.kind = k;
    input.checked = true;
    input.addEventListener("change", applyKindFilter);
    label.appendChild(input);
    label.appendChild(document.createTextNode(` ${k}`));
    fs.appendChild(label);
  }
}

function applyKindFilter() {
  document.querySelectorAll('input[data-kind]').forEach((cb) => {
    const kind = cb.dataset.kind;
    const display = cb.checked ? "element" : "none";
    cy.elements(`node[kind = "${kind}"]`).style("display", display);
  });
}

document.querySelectorAll('input[data-layer]').forEach((cb) => {
  cb.addEventListener("change", async () => {
    const memOn = document.querySelector('input[data-layer="memory"]').checked;
    const codeOn = document.querySelector('input[data-layer="code"]').checked;
    const layer = codeOn ? "all" : "memory";
    await loadSeed(layer);
    if (!memOn) {
      ["Memory", "Repo", "Author", "Tag"].forEach((k) => {
        cy.elements(`node[kind = "${k}"]`).style("display", "none");
      });
    }
  });
});

renderKindFilters();
```

- [ ] **Step 2: Manual smoke**

Toggle Code layer on; verify `File`/`Symbol` nodes appear. Uncheck `Tag`; verify tags disappear.
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add frontend/app.js
git commit -m "feat(frontend): kind filters + layer toggle"
```

---

## Task 25: Frontend `app.js` — search bar

**Files:**
- Modify: `frontend/app.js`

- [ ] **Step 1: Append search wiring**

```javascript
const qInput = document.getElementById("q");
const resultsEl = document.getElementById("results");
let searchTimer = null;

function renderResults(items) {
  resultsEl.replaceChildren();
  for (const r of items) {
    const li = document.createElement("li");
    li.textContent = `${r.kind}: ${r.label}`;
    li.dataset.id = r.id;
    li.addEventListener("click", async () => {
      resultsEl.hidden = true;
      try {
        const payload = await fetchJson(
          `/api/expand?id=${encodeURIComponent(r.id)}&depth=1`,
        );
        mergeElements(payload);
        runLayout(false);
        const ele = cy.getElementById(r.id);
        if (ele.nonempty()) {
          cy.center(ele);
        }
      } catch (e) {
        console.error(e);
      }
    });
    resultsEl.appendChild(li);
  }
  resultsEl.hidden = items.length === 0;
}

qInput.addEventListener("input", () => {
  if (searchTimer) clearTimeout(searchTimer);
  const q = qInput.value.trim();
  if (!q) {
    resultsEl.hidden = true;
    resultsEl.replaceChildren();
    return;
  }
  searchTimer = setTimeout(async () => {
    try {
      const data = await fetchJson(
        `/api/search?q=${encodeURIComponent(q)}&limit=20`,
      );
      renderResults(data.results || []);
    } catch (e) {
      console.error(e);
    }
  }, 200);
});

document.addEventListener("click", (e) => {
  if (!resultsEl.contains(e.target) && e.target !== qInput) {
    resultsEl.hidden = true;
  }
});
```

- [ ] **Step 2: Manual smoke**

Type `data` in the search box; click a hit; verify the node centres.
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add frontend/app.js
git commit -m "feat(frontend): search bar with debounce + click-to-centre"
```

---

## Task 26: Frontend `app.js` — detail panel + sanitised markdown

**Files:**
- Modify: `frontend/app.js`

- [ ] **Step 1: Append detail rendering**

```javascript
function buildBlock(title) {
  const block = document.createElement("div");
  block.className = "detail-block";
  const h = document.createElement("h3");
  h.textContent = title;
  block.appendChild(h);
  return block;
}

function buildBodyBlock(markdown) {
  const block = buildBlock("Body");
  const html = marked.parse(markdown);
  const fragment = DOMPurify.sanitize(html, { RETURN_DOM_FRAGMENT: true });
  block.appendChild(fragment);
  return block;
}

function buildEdgeBlock(title, edges, fieldKey) {
  const block = buildBlock(title);
  const ul = document.createElement("ul");
  ul.className = "edge-list";
  for (const e of edges) {
    const li = document.createElement("li");
    const other = e[fieldKey] || "";
    li.textContent = fieldKey === "target"
      ? `${e.edge_kind} → ${other}`
      : `${other} → ${e.edge_kind}`;
    ul.appendChild(li);
  }
  block.appendChild(ul);
  return block;
}

function renderDetail(detail) {
  const el = document.getElementById("detail");
  el.replaceChildren();

  const head = buildBlock(detail.node.kind);
  const idRow = document.createElement("p");
  const idLabel = document.createElement("strong");
  idLabel.textContent = "id: ";
  idRow.appendChild(idLabel);
  const idCode = document.createElement("code");
  idCode.textContent = detail.node.id;
  idRow.appendChild(idCode);
  head.appendChild(idRow);

  const labelRow = document.createElement("p");
  const labelLabel = document.createElement("strong");
  labelLabel.textContent = "label: ";
  labelRow.appendChild(labelLabel);
  labelRow.appendChild(document.createTextNode(detail.node.label));
  head.appendChild(labelRow);

  el.appendChild(head);

  if (detail.memory_body) {
    el.appendChild(buildBodyBlock(detail.memory_body));
  }
  if (detail.outbound && detail.outbound.length) {
    el.appendChild(buildEdgeBlock("Outbound", detail.outbound, "target"));
  }
  if (detail.inbound && detail.inbound.length) {
    el.appendChild(buildEdgeBlock("Inbound", detail.inbound, "source"));
  }
}

cy.on("tap", "node", async (evt) => {
  const node = evt.target;
  try {
    const detail = await fetchJson(
      `/api/node/${encodeURIComponent(node.id())}`,
    );
    renderDetail(detail);
  } catch (e) {
    console.error("detail failed", e);
  }
});
```

- [ ] **Step 2: Manual smoke**

Click a Memory node. Verify the panel shows body, id/label rows, and edge lists. Inspect the rendered DOM in DevTools — confirm there are no script tags inside the body block (DOMPurify strips them).
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add frontend/app.js
git commit -m "feat(frontend): node detail panel via sanitised DocumentFragment"
```

---

## Task 27: Frontend `app.js` — Reset key

**Files:**
- Modify: `frontend/app.js`

- [ ] **Step 1: Append the reset handler**

```javascript
async function resetView() {
  document.querySelectorAll('input[data-layer]').forEach((cb) => {
    cb.checked = cb.dataset.layer === "memory";
  });
  document.querySelectorAll('input[data-kind]').forEach((cb) => {
    cb.checked = true;
  });
  applyKindFilter();
  await loadSeed("memory");
  showDetailMessage("Click a node to see details.");
}

document.getElementById("reset").addEventListener("click", resetView);
document.addEventListener("keydown", (e) => {
  if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) {
    return;
  }
  if (e.key === "r" || e.key === "R") {
    resetView();
  }
});
```

- [ ] **Step 2: Manual smoke**

Toggle some filters, expand some nodes, then press `R`. Verify the view returns to the memory layer with all kinds on and the detail pane reset.
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add frontend/app.js
git commit -m "feat(frontend): R key resets layers/kinds/canvas"
```

---

## Task 28: Docs

**Files:**
- Modify: `docs/cli-reference.md`
- Modify: `docs/architecture.md`
- Modify: `README.md`

- [ ] **Step 1: Regenerate `docs/cli-reference.md` if a generator exists**

Run: `ls scripts/ | grep -i cli-ref` and `rg 'cli-reference' justfile 2>/dev/null`.
If a generator command exists (e.g. `just docs-cli` or `scripts/gen-cli-reference.sh`), run it after the new subcommand lands. Otherwise edit by hand.

- [ ] **Step 2: Add the `graph serve` section to `docs/cli-reference.md`**

```markdown
### `graph serve` — Local graph viewer

Spin up the read-only HTTP viewer for the kuzu property graph.

#### Flags

- `--port <PORT>` (default `0`) — bind port; `0` picks a free port.
- `--no-open` — skip auto-opening the URL.
- `--host <ADDR>` (default `127.0.0.1`) — bind address.
- `--bind-public` — required when `--host` is non-loopback.

#### Manual smoke checklist

1. `qwick-memory save -- "test memory body"` (or any existing memory).
2. `qwick-memory graph serve` — note the printed URL, browser opens automatically.
3. Default view shows the memory layer (`Memory`, `Repo`, `Author`, `Tag`).
4. Double-click a `Memory` node — neighbours appear.
5. Toggle the Code layer — `File` and `Symbol` nodes appear (if `qwick-memory index-code` has been run).
6. Type a tag name in the search box — pick a result; the node centres.
7. Click a `Memory` node — body and edges render in the detail panel.
8. Press `R` — view resets to the memory layer.
9. Ctrl-C — server shuts down cleanly.
```

- [ ] **Step 3: Update `docs/architecture.md`**

Find the existing module list and add:

```markdown
- **`serve/`** — local HTTP viewer for the property graph. Bound by
  `qwick-memory graph serve`. Holds a shared `Arc<Mutex<Graph>>`, exposes
  read-only REST endpoints (`/api/seed`, `/api/expand`, `/api/search`,
  `/api/node/{id}`), and embeds the vanilla-JS Cytoscape frontend from
  `frontend/` via `rust-embed`. Loopback-only by default.
```

- [ ] **Step 4: Update `README.md`**

Under Features (or equivalent), add:

```markdown
### Graph viewer

`qwick-memory graph serve` opens a local browser-based viewer for the
property graph. Click-to-expand neighbours, search across kinds, filter
by node kind, render memory bodies inline. Loopback-only; assets are
embedded in the binary.
```

- [ ] **Step 5: Typos gate**

Run: `bash scripts/typos-check.sh`
Expected: exit 0.

- [ ] **Step 6: Commit**

```bash
git add docs/cli-reference.md docs/architecture.md README.md
git commit -m "docs: cover qwick-memory graph serve viewer"
```

---

## Task 29: Final quality gate

**Files:** (verification only)

- [ ] **Step 1: Format**

Run: `bash scripts/fmt-check.sh`
Expected: exit 0.

- [ ] **Step 2: Type check**

Run: `bash scripts/type-check.sh`
Expected: exit 0.

- [ ] **Step 3: Lint**

Run: `bash scripts/lint-check.sh`
Expected: exit 0.

- [ ] **Step 4: Umbrella gate**

Run: `bash scripts/check-all.sh`
Expected: exit 0.

- [ ] **Step 5: Full test suite**

Run: `cargo nextest run --all-features`
Expected: every test passes.

- [ ] **Step 6: Optional umbrella QA**

Run: `just qa`
Expected: success, no new dup-check or deny-check warnings.

- [ ] **Step 7: Tag any small fixups**

If gates required tiny fixes (formatting, missing docstrings, a `query.rs` split because it grew past 500), commit as `chore: satisfy quality gates for graph viewer`.

---

## Self-Review

**Spec coverage walkthrough** (spec section → task):

- §3 user-facing surface (`graph serve` + flags) — Task 19.
- §4 architecture (axum + Arc<Mutex<Graph>> + embedded assets) — Tasks 2, 12, 13.
- §5 module layout (every file mapped) — Tasks 2, 3, 5, 12, 13, 14, 15, 16, 17.
- §6 deps — Task 1, with `mime_guess` adjustment in Task 12 and `nix`/`libc` in Task 20.
- §7.1 `/api/seed` — Task 14.
- §7.2 `/api/expand` — Task 15.
- §7.3 `/api/search` — Task 16.
- §7.4 `/api/node` — Task 17.
- §7.5 error envelope — Task 5.
- §8.1 layout — Task 21.
- §8.2 Cytoscape config — Task 22.
- §8.3 interactions — Tasks 22 (seed), 23 (double-click), 24 (filters/layer), 25 (search), 26 (detail), 27 (Reset).
- §8.4 markdown sanitisation — Task 21 (vendor) + Task 26 (DOMPurify `RETURN_DOM_FRAGMENT`).
- §9 security model — Task 5 (error envelope), Task 13 (CSP), Task 19 (host + `--bind-public`), Task 26 (sanitised markdown).
- §10 error handling (no unwrap/panic; tracing logs) — Task 5 + applied throughout.
- §11 testing — Tasks 5, 14–18, 20.
- §12 documentation — Task 28.
- §14 acceptance criteria — Tasks 19 (boot), 22 (seed render), 26 (detail), 23 (1-hop expand), 24 (code layer toggle), 25 (search), 29 (gates).

**Placeholder scan** — no `TBD`/`TODO`/`later` patterns. Tasks 6, 17 explicitly tell the engineer to locate existing helper names rather than invent them; that is a concrete instruction.

**Type consistency** — `NodeDto`, `EdgeDto`, `GraphPayload`, `SearchResult`, `SearchResponse`, `NodeDetail`, `EdgeRef`, `ApiError`, `ServerState`, `Graph::seed_memory_layer`, `Graph::seed_all`, `Graph::expand_neighbors`, `Graph::search_nodes`, `Graph::node_detail`, `serve::router::router`, `serve::router::run`, `serve::assets::serve_asset`, `loadSeed`, `mergeElements`, `runLayout`, `renderDetail`, `applyKindFilter`, `resetView`, `showDetailMessage` — referenced consistently.

**Innerhtml audit** — frontend tasks (21–27) use `replaceChildren`, `createElement`, `textContent`, `appendChild`, and `DOMPurify.sanitize(..., { RETURN_DOM_FRAGMENT: true })` for any sanitised markdown insertion. No `innerHTML` writes anywhere.

---

## Execution Handoff

**Plan complete and saved to `docs/superpowers/plans/2026-05-18-graph-visualization.md`. Two execution options:**

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration.

**2. Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints.

**Which approach?**
