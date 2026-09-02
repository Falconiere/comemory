# serve/

**What belongs here:** the loopback-only `comemory serve` HTTP server — axum
router assembly and the request-gating middleware, the versioned `/api/v1`
REST surface (`routes/`), the background-job model (`jobs/`), the response
envelope, graph-node-id-to-file resolution, the per-request repo scope, and
the per-session security primitives (bearer token, Host-header guard, path
containment).

**What does NOT belong here:** command logic. Every route calls an
`api::<cmd>::run` core — the same one the CLI calls — and never reimplements
ranking, indexing, or storage itself.

HTTP integration tests stay at crate-root: `tests/serve__routes__*.rs` per
resource, `tests/serve_scenario_*.rs` for multi-route journeys over a real
`comemory serve` (sharing `tests/common/serve_bin.rs`), catalogued in
`docs/scenarios/` — never under `src/serve/`.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `envelope.rs` | `Envelope` | The `{ok,data,meta}` / `{ok,error,meta}` `/api/v1` response envelope and the one `Error → (StatusCode, code)` mapping table every HTTP error (and every failed job) derives its status from |
| `jobs.rs` | `Registry` | The background job model for long-running commands; the table, spawner, and SSE event payloads live in `jobs/` |
| `repo_root.rs` | `resolve_root` | Resolve a `file:<repo>:<path>` graph node id to an absolute file on disk (also used by `retrieval::code_ref_fetch`) |
| `router.rs` | `build_router` | axum router assembly, the global body limit, and the path-aware request-gating middleware |
| `routes.rs` | `v1_router` | The versioned `/api/v1` surface: the aggregated route table and the handler-layer helpers every resource shares; per-resource files live in `routes/` |
| `scope.rs` | `RepoScope` | The per-request default `repo` filter: `X-Comemory-Repo` header first, the server's `--repo` second, never overriding an explicit parameter |
| `security.rs` | `generate_token` | Per-session bearer token, loopback Host-header guard, path containment |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/serve.rs` (`pub mod
<name>;`) and callers import concrete paths.
