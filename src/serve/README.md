# serve/

**What belongs here:** the loopback-only `comemory serve` HTTP server — axum
router assembly and the request-gating middleware, the versioned `/api/v1`
REST surface (`routes/`), the background-job model (`jobs/`), the response
envelope, graph-node-id-to-file resolution, the per-request repo scope,
and the per-session security primitives (bearer token, Host-header guard).
Path containment is transport-neutral and lives in
`utilities::path_containment`.

**What does NOT belong here:** command logic. Every route calls a
`domains::<capability>::<cmd>::run` core — the same one the CLI calls — and
never reimplements ranking, indexing, or storage itself. Nor CLI presentation:
since #178 nothing under `src/serve/` imports `cli::output`. The
`/api/v1/edges` body is built by `domains::graph::edges_result::envelope`, and
the `comemory serve` startup banner is written by `cli::serve` from the
`serve::Ready` value `serve::serve` hands its `ready` callback once the
listener is bound. The one deliberate exception is `routes/meta.rs`, which
imports `cli::{Cli, completion_script}`: `GET /completions` and
`GET /commands` are clap-introspection surfaces, so they read the clap
definition rather than forcing clap into a capability.

HTTP integration tests stay at crate-root: `tests/serve__routes__*.rs` per
resource, `tests/serve_scenario_*.rs` for multi-route journeys over a real
`comemory serve` (sharing `tests/common/serve_bin.rs`), catalogued in
`docs/scenarios/` — never under `src/serve/`.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `envelope.rs` | `Envelope` | The `{ok,data,meta}` / `{ok,error,meta}` `/api/v1` response envelope and the one `Error → (StatusCode, code)` mapping every HTTP error (and every failed job) derives its status from — the `code` and its class come from `utilities::error_code::classify`, shared with `mcp`; this file maps only `Class → StatusCode` |
| `jobs.rs` | `Registry` | The background job model for long-running commands; the table, spawner, and SSE event payloads live in `jobs/` |
| `router.rs` | `build_router` | axum router assembly, the global body limit, and the path-aware request-gating middleware |
| `routes.rs` | `v1_router` | The versioned `/api/v1` surface: the aggregated route table and the handler-layer helpers every resource shares; per-resource files live in `routes/` |
| `scope.rs` | `RepoScope` | The per-request default `repo` filter: `X-Comemory-Repo` header first, the server's `--repo` second, never overriding an explicit parameter |
| `security.rs` | `generate_token` | Per-session bearer token and the loopback Host-header guard (path containment is `utilities::path_containment`) |


`repo_root.rs` left this folder with
[#167](https://github.com/Falconiere/comemory/issues/167): resolving a
`file:<repo>:<path>` id to a file on disk is not an HTTP concern, and the CLI,
retrieval freshness and reference refresh needed the same resolver. It is
[`utilities::repo_root`](../utilities/README.md) now, and `serve` calls it
like every other caller.

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/serve.rs` (`pub mod
<name>;`) and callers import concrete paths.
