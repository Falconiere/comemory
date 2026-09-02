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
| `context.rs` | `Request` | Shared middle of `comemory context` / `GET\|POST /api/v1/context`. `query` accepts `key` as a serde alias, the spelling the console-api spec's `GET /context?key=` uses |
| `delete.rs` | `Response` | Shared middle of `comemory delete` / `DELETE /api/v1/memories/{id}` |
| `doctor.rs` | `Request` | Shared middle of `comemory doctor` / `GET /api/v1/doctor`; the individual health probes live in `doctor/checks.rs` |
| `find.rs` | `Request` | Shared middle of `comemory find` / `GET\|POST /api/v1/find` — the unified memory + code + document ranking |
| `hooks.rs` | `Request` | Shared middle of `comemory hooks` / `GET\|POST /api/v1/hooks` — per-hook read and toggle, state read from `.git/hooks` |
| `edges.rs` | `Request` | Shared middle of `comemory edges` / `GET /api/v1/edges` |
| `eval.rs` | `Request` | Shared middle of `comemory eval` / `POST /api/v1/eval` |
| `feedback.rs` | `Request` | Shared middle of `comemory feedback` / `POST /api/v1/feedback` |
| `gc.rs` | `Request` | Shared middle of `comemory gc` / `POST /api/v1/gc` — reaps aged `.trash/` files AND purges their mirror rows (`store::memory_purge`), healing zombie rows earlier sweeps left behind |
| `graph.rs` | `Request` | Shared middle behind `comemory graph` / `GET /api/v1/graph` |
| `index.rs` | `Request` | Shared middle of `comemory index` / `POST /api/v1/index` |
| `index_code.rs` | `Request` | Shared middle of `comemory index-code` / `POST /api/v1/code/index` (+ `POST /index/runs`), incl. the `mode` (incremental\|full) switch and the `index_runs` row every run records; the walk internals live in `index_code/` |
| `ingest_code.rs` | `Response` | Shared middle of `comemory ingest-code` / `POST /api/v1/code/ingest` |
| `install_hooks.rs` | `Request` | Shared middle of `comemory install-hooks` / `POST /api/v1/hooks/install` |
| `list.rs` | `Request` | Shared middle of `comemory list` / `GET /api/v1/memories` |
| `mine.rs` | `Request` | Shared middle of `comemory mine` / `POST /api/v1/mine` |
| `prune.rs` | `Request` | Shared middle of `comemory prune` / `GET\|POST /api/v1/prune` |
| `rebuild.rs` | `Request` | Shared middle of `comemory rebuild` / `POST /api/v1/rebuild`; the preservation copy lives in `rebuild/` |
| `save.rs` | `Request` | Shared middle of `comemory save` / `POST /api/v1/memories` |
| `search.rs` | `Request` | Shared middle of `comemory search` / `GET\|POST /api/v1/memories/search` |
| `search_code.rs` | `Request` | Shared middle of `comemory search-code` / `GET\|POST /api/v1/code/search` |
| `repos.rs` | `Request` | Shared middle of `comemory repos` / `GET /api/v1/repos`; the git probes live in `repos/git_state.rs` |
| `show.rs` | `Request` | Shared middle of `comemory show` / `GET /api/v1/memories/{id}` — one memory in full |
| `sources.rs` | `Request` | Shared middle of `comemory sources` / the `/api/v1/sources` routes |
| `stats.rs` | `Request` | Shared middle of `comemory stats` / `GET /api/v1/stats` — corpus counters and database size |
| `tune.rs` | `Request` | Shared middle of `comemory tune` / `POST /api/v1/tune` |
| `unindex.rs` | `Request` | Shared middle of `comemory unindex` / the document-unindex route |
| `config_retrieval.rs` | `RetrievalKnobs` | Console-only: `GET\|PUT /api/v1/config/retrieval` — the live ranking knobs with their ranges, and the validated partial update |
| `gc_policy.rs` | `Policy` | Console-only: `GET\|PUT /api/v1/gc/policy` — trash + telemetry retention windows and the last gc run |
| `graph_nodes.rs` | `NodeDetail` | Console-only: `GET /api/v1/graph/nodes`, `/graph/nodes/{id}`, `/graph/nodes/{id}/neighbors`, `/graph/snapshot` |
| `graph_recompute.rs` | `Response` | Console-only: `POST /api/v1/graph/recompute` — PageRank re-projection for every repo, then the memory rank |
| `index_runs.rs` | `Request` | Console-only: `GET /api/v1/index/runs` — the paged `index_runs` history |
| `learning.rs` | `Summary` | Console-only: `GET /api/v1/learning/{summary,evals,golden-set,expansions}` |
| `learning_proposals.rs` | `Proposal` | Console-only: knob proposals derived from unapplied `tune`/`bandit` runs — list, apply (writes `config.toml`), discard |
| `memory_store.rs` | `Store` | Console-only: the one memory store's view, its `[git]` patch, and the `store-sync` job (commit → pull → push); the `git` shell-outs live in `memory_store/git.rs` |
| `overview.rs` | `Response` | Console-only: `GET /api/v1/overview` (+ `/overview/eval-series`) — counters, index state, last run, metrics, recent memories |
| `reembed.rs` | `Request` | Console-only: `POST /api/v1/doctor/reembed` — re-vectorize memories and/or code through the embed command, cancellable |
| `refresh_refs.rs` | `Response` | Console-only: `POST /api/v1/memories/{id}/references/refresh` — re-pin anchored code references to the current HEAD |
| `repo_admin.rs` | `ConnectRequest` | Console-only: `POST /api/v1/repos`, `PATCH /repos/{name}`, `POST /repos/{name}/archive`, `DELETE /repos/{name}` |
| `restore.rs` | `Response` | Console-only: `POST /api/v1/memories/{id}/restore` / `POST /trash/{id}/restore` — bring a soft-deleted memory back from `.trash/`, re-deriving the incoming relation edges soft-delete dropped from the live tree's frontmatter; a mirror failure names the path and the `rebuild` recovery |
| `suggest.rs` | `Request` | Console-only: `GET /api/v1/search/suggest` — mined expansions + recent queries for the ⌘K palette |
| `trash.rs` | `TrashRow` | Console-only: `GET /api/v1/trash` — soft-deleted memories with their days until gc |
| `update.rs` | `Request` | Console-only: `PATCH /api/v1/memories/{id}` — frontmatter patch in place, body patch as a superseding re-save |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/api.rs` (`pub mod
<name>;`) and callers import concrete paths.
