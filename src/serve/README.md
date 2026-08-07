# serve/

**What belongs here:** the loopback-only `comemory serve` HTTP server — axum
router assembly and request handlers, the embedded React/Vite SPA, on-disk
file read/write for the in-browser editor, the `/api/search` bridge into
retrieval, graph-node-id-to-file resolution, and the per-session security
primitives (bearer token, Host-header guard, path containment).

**What does NOT belong here:** ranking logic. `serve::search` calls
`retrieval::code_search`; it coalesces symbol hits to file level and never
reimplements BM25/ANN/rerank itself.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `assets.rs` | `WebAssets` | Embedded React/Vite frontend assets |
| `error.rs` | `ApiError` | Map the crate `Error` to an HTTP response |
| `fileio.rs` | `MAX_FILE_BYTES` | Read and write indexed source files for the editor (`PUT /api/file`, `--read-only` → 405) |
| `handlers.rs` | `FileQuery` | Async request handlers for the `comemory serve` API |
| `repo_root.rs` | `resolve_root` | Resolve a `file:<repo>:<path>` graph node id to an absolute file on disk |
| `router.rs` | `build_router` | axum router assembly + request-gating middleware |
| `search.rs` | `FileHit` | `GET /api/search?q=<phrase>&k=<n>` — ranked file hits for the web viewer |
| `security.rs` | `generate_token` | Per-session bearer token, loopback Host-header guard, path containment |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/serve.rs` (`pub mod
<name>;`) and callers import concrete paths.
