# api/

**What belongs here:** the shared middle of every `comemory` subcommand.
`api::<cmd>::run(&mut Ctx, Request) -> Result<Response>` holds the logic that
both surfaces need, so `cli::<cmd>` and `serve::routes::<resource>` call one
implementation instead of keeping two in step. One file per subcommand, named
after the subcommand.

**What does NOT belong here:** argument parsing and rendering. clap `Args`
structs, TTY colouring and `--json` emission stay in `cli/`; HTTP status
mapping, the response envelope and the read-only/confirm gates stay in
`serve/routes/`. An `api::` module takes a plain `Request` and returns a plain
`Response` — it never touches `stdout` and never names an HTTP type.

`Ctx` (in `src/api.rs`) bundles `Paths` + `Config` with a connection that is
either `Borrowed` (the CLI's own connection, or the server's shared
per-request one) or `Lazy` (opened on first `Ctx::conn()` call — a job
worker's own dedicated connection). Conn-free commands (`doctor`, `rebuild`,
`ast`, `install-hooks`, `completions`) never open one at all.

Every `Request` derives `#[serde(deny_unknown_fields)]`, enforced by the
clap-introspection walk in `tests/api__parity.rs`.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `ast.rs` | `Request` | Shared middle of `comemory ast` / `POST /api/v1/code/ast` |
| `bandit.rs` | `Request` | Shared middle of `comemory bandit` / `POST /api/v1/bandit` |
| `completions.rs` | `Request` | Shared middle of `comemory completions` / `GET /api/v1/completions` |
| `consolidate.rs` | `Request` | Shared middle of `comemory consolidate` / `GET /api/v1/consolidate` |
| `context.rs` | `Request` | Shared middle of `comemory context` / `GET\|POST /api/v1/context` |
| `delete.rs` | `Response` | Shared middle of `comemory delete` / `DELETE /api/v1/memories/{id}` |
| `doctor.rs` | `Request` | Shared middle of `comemory doctor` / `GET /api/v1/doctor` |
| `edges.rs` | `Request` | Shared middle of `comemory edges` / `GET /api/v1/edges` |
| `eval.rs` | `Request` | Shared middle of `comemory eval` / `POST /api/v1/eval` |
| `feedback.rs` | `Request` | Shared middle of `comemory feedback` / `POST /api/v1/feedback` |
| `gc.rs` | `Request` | Shared middle of `comemory gc` / `POST /api/v1/gc` |
| `graph.rs` | `Request` | Shared middle behind `comemory graph` / `GET /api/v1/graph` |
| `index.rs` | `Request` | Shared middle of `comemory index` / `POST /api/v1/index` |
| `index_code.rs` | `Request` | Shared middle of `comemory index-code` / `POST /api/v1/code/index`; the walk internals live in `index_code/` |
| `ingest_code.rs` | `Response` | Shared middle of `comemory ingest-code` / `POST /api/v1/code/ingest` |
| `install_hooks.rs` | `Request` | Shared middle of `comemory install-hooks` / `POST /api/v1/hooks/install` |
| `list.rs` | `Request` | Shared middle of `comemory list` / `GET /api/v1/memories` |
| `mine.rs` | `Request` | Shared middle of `comemory mine` / `POST /api/v1/mine` |
| `prune.rs` | `Request` | Shared middle of `comemory prune` / `GET\|POST /api/v1/prune` |
| `rebuild.rs` | `Request` | Shared middle of `comemory rebuild` / `POST /api/v1/rebuild`; the preservation copy lives in `rebuild/` |
| `save.rs` | `Request` | Shared middle of `comemory save` / `POST /api/v1/memories` |
| `search.rs` | `Request` | Shared middle of `comemory search` / `GET\|POST /api/v1/memories/search` |
| `search_code.rs` | `Request` | Shared middle of `comemory search-code` / `GET\|POST /api/v1/code/search` |
| `sources.rs` | `Request` | Shared middle of `comemory sources` / the `/api/v1/sources` routes |
| `tune.rs` | `Request` | Shared middle of `comemory tune` / `POST /api/v1/tune` |
| `unindex.rs` | `Request` | Shared middle of `comemory unindex` / the document-unindex route |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/api.rs` (`pub mod
<name>;`) and callers import concrete paths.
