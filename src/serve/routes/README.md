# serve/routes/

**What belongs here:** the versioned `/api/v1` REST surface — one file per
resource, each exposing a `router()` and a `table_entries()` that
`src/serve/routes.rs` aggregates into the single route table (method, path,
CLI command, `mutating` flag). That table is the source of truth for the
read-only gate, `GET /commands`, and the parity test, so a route that is not
in it does not exist.

`src/serve/routes.rs` also owns the handler-layer helpers every resource
shares: `run_blocking` (runs `api::<cmd>::run` and takes the connection mutex
entirely inside one `spawn_blocking` closure, never across an `.await`),
`respond`/`accepted`, `guard_mutating`, `guard_job`, `require_confirm`, and
`track_for`.

**What does NOT belong here:** command logic, which is `api::`'s, and the
legacy un-versioned handlers, which stay in `src/serve/handlers.rs`.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `code.rs` | `table_entries` | `GET\|POST /code/search`, `POST /code/ast`, and the job-backed `POST /code/index` / `POST /code/ingest` under their own body-limit layer |
| `find.rs` | `table_entries` | `GET\|POST /find` — the unified ranking. Its own resource because it is cross-domain, not a memories sub-resource |
| `graph.rs` | `table_entries` | `GET /graph` and `GET /edges`, reusing the legacy graph builders — no second query path |
| `hooks.rs` | `table_entries` | `GET /hooks` (read) and `POST /hooks` (per-hook toggle, read-only gated, not confirm-gated) |
| `jobs.rs` | `table_entries` | `GET /jobs`, `GET /jobs/{id}`, and the `GET /jobs/{id}/events` SSE stream |
| `learning.rs` | `table_entries` | Job-backed `POST /eval` (read class) plus `POST /tune` and `POST /bandit`, confirm-gated only when `apply` |
| `maint.rs` | `table_entries` | `GET /doctor` and `GET /consolidate`; the rest of the maintenance surface lives in `maint/` |
| `memories.rs` | `table_entries` | `GET /memories` and `GET /memories/{id}`; search and write live in `memories/` |
| `meta.rs` | `table_entries` | `GET /completions` and `GET /commands` — the clap-introspected route/command inventory |
| `repos.rs` | `table_entries` | `GET /repos` — the indexed code-repository inventory |
| `sources.rs` | `table_entries` | `GET /sources`, job-backed `POST /sources`, and `DELETE /sources?target=&confirm=` |
| `stats.rs` | `table_entries` | `GET /stats` — corpus counters and database size |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/serve/routes.rs` (`pub
mod <name>;`) and callers import concrete paths.
