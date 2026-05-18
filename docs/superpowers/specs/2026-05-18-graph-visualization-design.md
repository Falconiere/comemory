# Graph Visualization Design

- **Date:** 2026-05-18
- **Status:** Draft (awaiting plan)
- **Author:** Falconiere (hello@falconiere.io)
- **Related modules:** `src/graph/`, `src/cli/`, new `src/serve/`

## 1. Goal

Add an interactive, browser-based viewer for the kuzu property graph that
backs `qwick-memory`. The viewer must let a user explore both the memory
layer (`Memory`, `Repo`, `Author`, `Tag`) and the code layer (`File`,
`Symbol`) along with the relations between them, expand neighborhoods on
demand, filter by node and edge kind, search by id/name/tag, and inspect
the underlying memory body when a `Memory` node is selected.

The viewer ships inside the existing `qwick-memory` binary, runs locally
on loopback only, and requires no external services.

## 2. Non-goals

- Mutating the graph from the viewer. v1 is strictly read-only — no
  POST/PUT/DELETE endpoints, no edit UI.
- Live push of graph changes (WebSocket / SSE). Deferred to v2.
- Multi-user / remote access. Loopback only by default; non-loopback bind
  requires an explicit second flag and a logged warning.
- Built-in authentication. Loopback is the trust boundary.
- Graph analytics (centrality, community detection, etc.). Future work.
- Mobile / responsive layout. Desktop browser only.
- Frontend test infrastructure (Playwright, Jest, etc.). Manual smoke for
  v1; backend tests cover the contract.

## 3. User-facing surface

A new top-level subcommand `qwick-memory graph` carrying a single
sub-subcommand `serve`:

```
qwick-memory graph serve [--port <PORT>] [--no-open] [--host <ADDR>] [--bind-public]
```

| Flag | Default | Meaning |
|------|---------|---------|
| `--port` | `0` | Bind port. `0` lets the kernel pick a free ephemeral port. |
| `--no-open` | off | Do not auto-open the URL in a browser. |
| `--host` | `127.0.0.1` | Bind address. Loopback by default. |
| `--bind-public` | off | Required when `--host` is non-loopback. Prints a warning + emits a `tracing::warn!` line. |

`graph` is added as a parent subcommand (modelled after how clap nests
sub-subcommands elsewhere in this CLI) so future siblings such as
`graph export` or `graph stats` can slot in without renaming.

On start the binary prints:

```
qwick-memory graph viewer
  listening on http://127.0.0.1:54812
  data-dir: /Users/.../.qwick-memory
  press Ctrl-C to stop
```

It then blocks until SIGINT/SIGTERM, gracefully shutting down the axum
server.

## 4. Architecture

```
qwick-memory graph serve
        │
        ▼
┌─────────────────────────┐         ┌────────────────────┐
│ axum on <host>:<port>   │◀── HTTP ┤ Browser            │
│  ├─ GET /  (HTML+JS)    │         │  Cytoscape.js      │
│  ├─ GET /api/seed       │         │  - filters         │
│  ├─ GET /api/expand     │         │  - search bar      │
│  ├─ GET /api/search     │         │  - detail panel    │
│  └─ GET /api/node/{id}  │         └────────────────────┘
│         │               │
│         ▼               │
│  Arc<Mutex<Graph>>      │
│  (kuzu connection)      │
└─────────────────────────┘
        │
        ▼
   ~/.qwick-memory/kuzu/
```

The server holds a single shared `Arc<Mutex<Graph>>` that wraps the
existing `crate::graph::Graph` handle. kuzu's `Connection` is not `Sync`,
so all queries serialise behind the mutex. Latency budget per click is
"good enough for human interaction" (≤200ms p95 on a memory layer with a
few hundred nodes); a fancier connection pool is not warranted in v1.

Static assets (HTML, CSS, JS, vendored libraries) are embedded into the
binary at build time via `rust-embed` so the single-binary install story
remains intact.

The query strategy is **live Cypher per request** — no in-memory snapshot
or cache. The kuzu store is fast enough on the memory-layer scope, and
this avoids any cache-invalidation bug after a concurrent
`qwick-memory save`.

## 5. Module layout

```
src/
├── cli/
│   └── graph_serve.rs        # subcommand entry, port wiring, browser open
├── serve/
│   ├── mod.rs                # pub use; module docstring
│   ├── router.rs             # axum Router + ServerState
│   ├── assets.rs             # rust-embed of frontend/dist/*
│   ├── error.rs              # ApiError → IntoResponse, JSON error shape
│   ├── dto.rs                # NodeDto, EdgeDto, ExpandResponse, …
│   └── handlers/
│       ├── mod.rs
│       ├── seed.rs           # GET /api/seed?layer=…
│       ├── expand.rs         # GET /api/expand?id=&depth=
│       ├── search.rs         # GET /api/search?q=&limit=
│       └── node.rs           # GET /api/node/{id}
├── graph/
│   └── query.rs              # adds pub(crate) fns used by handlers
frontend/                     # source-only, not in src/
├── index.html
├── app.js
├── styles.css
└── vendor/
    ├── cytoscape.min.js
    ├── cytoscape-cose-bilkent.min.js
    ├── layout-base.min.js
    ├── cose-base.min.js
    ├── marked.min.js
    ├── purify.min.js         # DOMPurify
    └── README.md             # versions + SHA-256 checksums

tests/
├── serve.rs                  # thin shim: declares submodules in tests/serve/
├── cli.rs                    # existing shim, gains `mod graph_serve;`
├── serve/
│   ├── router.rs
│   ├── assets.rs
│   ├── error.rs
│   ├── dto.rs
│   ├── handlers.rs           # declares submodules in tests/serve/handlers/
│   └── handlers/
│       ├── seed.rs
│       ├── expand.rs
│       ├── search.rs
│       └── node.rs
└── cli/
    └── graph_serve.rs
```

`frontend/` lives at the repo root, outside `src/`, so the 500-line
module-size check does not apply to vendored JS. `rust-embed` reads from
`frontend/` at build time. The vendor directory's `README.md` documents
the upstream URL, version, and a SHA-256 checksum for each file so the
provenance is reviewable.

Anticipated line counts per file (≤500 each):

| File | Approx LoC |
|------|-----------|
| `src/cli/graph_serve.rs` | 120 |
| `src/serve/router.rs` | 150 |
| `src/serve/assets.rs` | 40 |
| `src/serve/error.rs` | 80 |
| `src/serve/dto.rs` | 120 |
| `src/serve/handlers/*.rs` | 80 each |
| `src/graph/query.rs` (after additions) | ~210 |

If `src/graph/query.rs` later approaches 500 lines, split it into
`graph/query/memory.rs` + `graph/query/code.rs` re-exported via a new
`graph/query/mod.rs`. Not needed in v1.

## 6. New dependencies

Added to `Cargo.toml`:

- `axum` — HTTP router. Version pinned at the latest 0.7.x line.
- `tokio` — async runtime. Already pulled transitively by lancedb; add an
  explicit dep with the `rt-multi-thread`, `macros`, `signal` features.
- `tower` — required by axum.
- `tower-http` — features `compression-gzip`, `trace`, `set-header`.
  Used for `CompressionLayer`, `TraceLayer` (logs each request at
  `tracing::debug!`), and `SetResponseHeaderLayer` to attach the CSP
  header to `GET /`.
- `rust-embed` — compile static assets into the binary.
- `open` — cross-platform browser launcher (used unless `--no-open`).
- `reqwest` (dev-dep only, `blocking` feature) — used by the CLI
  integration test to hit endpoints.

Existing `serde`, `serde_json`, `clap`, `tracing` are reused. No new
runtime dep is added that needs an OS package; the binary remains a
single static artefact on each supported target.

## 7. REST API contract

All responses are JSON. The wire-level node id is namespaced so the
frontend can style by prefix and the backend can route to the right
table.

| Prefix | kuzu table | Id payload |
|--------|-----------|------------|
| `m:` | `Memory` | `Memory.id` (8 hex) |
| `r:` | `Repo` | `Repo.name` |
| `a:` | `Author` | `Author.name` |
| `t:` | `Tag` | `Tag.name` |
| `f:` | `File` | `File.qualified` |
| `s:` | `Symbol` | `Symbol.qualified` |

### 7.1 `GET /api/seed?layer=memory|all`

Returns the initial graph payload. Default `layer=memory` returns only
`Memory`, `Repo`, `Author`, `Tag` nodes plus memory-layer edges. `all`
additionally returns every `File` and `Symbol` node and the cross-layer
`ReferencesFile` / `ReferencesSymbol` edges.

Response:

```json
{
  "nodes": [
    {
      "id": "m:a1b2c3d4",
      "label": "a1b2c3d4",
      "kind": "Memory",
      "props": { "quality": 4, "created": "2026-05-17T14:30:00Z" }
    }
  ],
  "edges": [
    {
      "id": "e:7c1f0a4e3b9d5821",
      "source": "m:a1b2c3d4",
      "target": "r:qwick-backend",
      "kind": "InRepo",
      "props": {}
    }
  ]
}
```

Edge ids are deterministic, opaque strings: the backend computes
`format!("e:{:016x}", siphash(source, kind, target))` using a
fixed-key siphasher (no randomised seed). Stable across calls and across
processes for the same input triple. The frontend must treat edge ids as
opaque — it never parses them. This avoids ambiguity when `source` or
`target` themselves contain colons (e.g. `s:repo:path/to/file.rs:func`).

### 7.2 `GET /api/expand?id=<ns:id>&depth=1`

Returns nodes and edges within `depth` hops of `id`, in both directions.
`depth` is clamped to `1..=3`. The response shape is identical to
`/api/seed`. The seed node itself is included so the frontend can
deduplicate by id.

### 7.3 `GET /api/search?q=<term>&limit=20`

Substring (case-insensitive) match across `Memory.id`, `Tag.name`,
`Author.name`, `Repo.name`, `Symbol.name`, `File.path`. `limit` is
clamped to `1..=100`. `q` is required and length-capped at 128 chars.

Response:

```json
{
  "results": [
    { "id": "m:a1b2c3d4", "label": "a1b2c3d4", "kind": "Memory" }
  ]
}
```

Edges are not returned by `/api/search`; the frontend follows up with
`/api/expand` once a result is chosen.

### 7.4 `GET /api/node/{ns:id}`

Full detail for a single node.

Response:

```json
{
  "node": { "id": "m:a1b2c3d4", "label": "…", "kind": "Memory", "props": { … } },
  "memory_body": "markdown content …",
  "frontmatter": { "id": "a1b2c3d4", "kind": "decision", "tags": ["…"], "…": "…" },
  "outbound": [
    { "edge_kind": "InRepo", "target": "r:qwick-backend" }
  ],
  "inbound": [
    { "edge_kind": "ReferencesFile", "source": "m:…" }
  ]
}
```

`memory_body` and `frontmatter` are present only when `node.kind ==
"Memory"`; the markdown is fetched via the existing `crate::memory::load`
helper. For all other kinds those fields are omitted.

### 7.5 Error envelope

```json
{
  "error": {
    "code": "not_found",
    "message": "node m:zzzzzzzz not found"
  }
}
```

| `code` | HTTP | Trigger |
|--------|------|---------|
| `not_found` | 404 | unknown node id, missing memory file |
| `invalid_param` | 400 | bad `layer`, out-of-range `depth`/`limit`, missing `q`, oversize `q` |
| `graph_error` | 500 | kuzu query failure (logged via `tracing::error!`) |
| `io_error` | 500 | filesystem failure when loading a memory body |

## 8. Frontend behaviour

Vanilla JS — no React, no bundler, no build step. A single `app.js` (<
400 LoC target) wires Cytoscape, the filter checkboxes, the search box,
and the detail panel.

### 8.1 Layout

```
┌───────────────────────────────────────────────────────────────┐
│ qwick-memory graph                            [search box  🔍] │
├──────────┬─────────────────────────────────┬──────────────────┤
│ FILTERS  │                                 │ DETAIL           │
│ Layers   │      Cytoscape canvas           │  …               │
│ Kinds    │      (pan / zoom / drag)        │                  │
│ Edges    │                                 │                  │
│ Reset    │                                 │                  │
└──────────┴─────────────────────────────────┴──────────────────┘
```

### 8.2 Cytoscape configuration

- Layout: `cose-bilkent` (force-directed). Re-run with `animate: false`
  on incremental adds so the existing graph does not violently reflow.
- Style: node color by `kind` (Memory blue, Repo green, Author purple,
  Tag gray, File orange, Symbol red). Edge color by `kind`. Node label
  is `props.label || id` truncated to 24 chars.
- Layer toggle is a Cytoscape selector visibility toggle, not a refetch.

### 8.3 Interactions

| User action | Behaviour |
|-------------|-----------|
| Initial load | `GET /api/seed?layer=memory` → render → run layout |
| Toggle Code layer on | `GET /api/seed?layer=all` → merge new elements → re-run layout |
| Toggle a kind checkbox | Client-side `display: none` via selector; no refetch |
| Double-click node | `GET /api/expand?id=…&depth=1` → merge → layout |
| Single-click node | `GET /api/node/{id}` → render detail panel |
| Click edge target in detail panel | `cy.center` if loaded, else expand first |
| Type in search box (200 ms debounce) | `GET /api/search?q=…&limit=20` → dropdown |
| Pick search result | `/api/expand?id=…&depth=0` (or 1) → centre |
| Press `R` | `Reset` — drop all loaded elements, reset layer toggle and kind filters to defaults (memory layer on, code layer off, all kinds visible), refetch `/api/seed?layer=memory`, re-layout |

`depth=0` is rejected by the backend; the frontend uses `depth=1` and
then centres the seed node, which gives the same visual effect with less
special-casing in handlers.

### 8.4 Markdown rendering

Memory body in the detail panel is rendered with `marked.min.js` and the
HTML output is run through `DOMPurify.sanitize(...)` before being assigned
to `innerHTML`. `marked`'s own `sanitize` option was removed in v7, so
DOMPurify is the supported path. Both libraries are vendored under
`frontend/vendor/` with pinned versions and SHA-256 checksums.

This neutralises raw HTML in any saved memory body so a malicious-looking
note cannot inject script tags into the viewer.

## 9. Security model

- **Default bind** `127.0.0.1` only. `--host ::1` is also accepted as
  loopback. Any other value requires `--bind-public`; a `tracing::warn!`
  fires and the startup banner adds a "⚠ public bind" line.
- **No auth.** The trust boundary is the loopback socket. Documented on
  `--bind-public`.
- **CORS.** No `Access-Control-Allow-Origin` header is emitted. Same
  origin only.
- **CSP** on `/`:
  `default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'`.
  `unsafe-inline` for styles only — Cytoscape sets inline styles on
  elements at runtime.
- **Cypher injection.** All user input passed to kuzu (`id`, `q`,
  `layer`) is parameterised via kuzu prepared statements where available,
  otherwise routed through `crate::graph::upsert::esc`. Numeric inputs
  (`depth`, `limit`) parse as `u32` and are clamped before reaching
  Cypher.
- **Input limits.** `q` ≤ 128 chars, `limit` ∈ `1..=100`, `depth` ∈
  `1..=3`. Anything outside → 400.
- **Read-only.** No POST/PUT/DELETE handlers exist. The viewer cannot
  mutate the graph or the markdown store.
- **Markdown sanitisation.** Memory bodies are rendered with
  HTML-sanitising markdown, so a body containing `<script>` becomes
  literal text.

## 10. Error handling

`src/serve/error.rs` defines:

```rust
pub(crate) struct ApiError {
    code: &'static str,
    status: StatusCode,
    message: String,
}
```

with `impl IntoResponse for ApiError` rendering the JSON envelope from
§7.5, and `impl From<crate::Error> for ApiError` mapping the existing
`Error` enum to the right code/status. Handlers return
`Result<Json<T>, ApiError>`.

No `.unwrap()`, no `panic!`, no `println!`/`eprintln!`. Failures are
logged with `tracing::error!` and surfaced to the client as the envelope
above. Internal log lines are not leaked into the `message` field.

## 11. Testing strategy

All tests live under `tests/`, mirroring `src/` 1:1, per binding rule 5.

### 11.1 Per-handler tests

Each `tests/serve/handlers/<name>.rs` boots a `Router` against a
temp-dir `Graph` built by helpers in `tests/common/`, then exercises the
route via `tower::ServiceExt::oneshot`. Cases:

- `seed.rs` — `layer=memory` returns expected node count and kinds; bad
  `layer` → 400.
- `expand.rs` — `depth=1` returns direct neighbors only;
  `depth=4` → 400; unknown id → 404; round-trip preserves edge ids.
- `search.rs` — substring match across kinds; `limit` clamping;
  oversize `q` → 400.
- `node.rs` — `Memory` returns body + frontmatter; unknown id → 404;
  `File` node returns no body.

### 11.2 Wiring + plumbing tests

- `tests/serve/router.rs` — every declared route resolves; unknown route
  → 404.
- `tests/serve/dto.rs` — round-trip serde for `NodeDto`, `EdgeDto`,
  `ExpandResponse`, `SearchResult`, `NodeDetail`.
- `tests/serve/error.rs` — each `crate::Error` variant maps to the
  documented `ApiError`.
- `tests/serve/assets.rs` — embedded `/`, `/app.js`, `/styles.css`,
  vendor files are non-empty and have expected content-type.

### 11.3 CLI integration

`tests/cli/graph_serve.rs` uses `assert_cmd` + `reqwest::blocking`:

1. Spawn `qwick-memory graph serve --port 0 --no-open` with a temp
   `QWICK_MEMORY_DATA_DIR`.
2. Parse the printed `listening on http://...` line.
3. `GET /api/seed?layer=memory` → 200 + a valid envelope.
4. Send SIGTERM (or kill the child) and assert clean shutdown within 2s.

### 11.4 Fixtures

A new `tests/common/graph_fixture.rs` builds a known sub-graph: three
memories, two repos, one shared tag, one supersession, one conflict,
plus a File + Symbol referenced by one memory. All handler tests share
this fixture so assertions are stable and small.

### 11.5 Frontend

Out of scope for v1. The contract is covered by backend tests; a
documented manual smoke-test checklist lives in `docs/cli-reference.md`
under the `graph serve` section.

## 12. Documentation

- Update `docs/cli-reference.md` with `graph serve` flags, the printed
  URL behaviour, and the manual smoke checklist.
- Update `docs/architecture.md` to mention the new `serve/` module and
  the embedded frontend.
- Update `README.md` "Known v1.x gaps" / "Features" sections with a
  one-paragraph mention.
- `frontend/vendor/README.md` records each vendored file's upstream URL,
  version, and SHA-256 checksum.

## 13. Open questions / future work

- Live updates: file-watch on `~/.qwick-memory/memories/**` could push
  graph deltas via SSE. Deferred; the manual `R` reset key covers v1.
- Layout: `cose-bilkent` is a reasonable default. If user feedback wants
  layered Sugiyama for supersession chains, add a layout picker.
- Export: `graph export --format dot|json|graphml` would round out the
  story for users who want to feed Gephi/yEd. Deferred.
- Multi-graph: today there is one `Graph` per data-dir. If we ever ship
  named graphs, `serve` will need a `--name` flag.

## 14. Acceptance criteria

A reviewer is satisfied when, on a fresh machine with `cargo install
--path .`:

1. `qwick-memory graph serve` boots, prints a `127.0.0.1` URL, and opens a
   browser to that URL (unless `--no-open`).
2. The page renders the memory layer of the current data-dir.
3. Clicking a `Memory` node fills the detail panel with the body and
   edge lists fetched from `/api/node/{id}`.
4. Double-clicking a node loads its 1-hop neighbours.
5. Toggling the Code layer brings `File` and `Symbol` nodes into the
   view.
6. The search box matches across memory ids, tags, authors, repos,
   files, and symbols.
7. `bash scripts/check-all.sh` exits 0.
8. `cargo nextest run --all-features` exits 0.
