# The `/api/v1` HTTP API

**Goal:** drive every `comemory` subcommand over HTTP — from a local agent, an
editor extension, a console, or a script — through the loopback server
`comemory serve` starts, with no second copy of any command's logic.

## What it is

`comemory serve` is a loopback-only HTTP server whose whole surface is the
versioned REST API at `/api/v1` — there is no bundled web page. Almost
every CLI subcommand — `save`, `search`, `search-code`, `index-code`,
`eval`, `rebuild`, and 20-odd more — gets a `/api/v1` route. Both surfaces
share one command core, `src/api/`: each `api::<cmd>::run(&mut Ctx, Request)`
holds a subcommand's logic once, called by `cli::<cmd>::run` (which adds
arg-parsing and TTY/`--json` rendering) and by the matching HTTP handler
(which adds JSON (de)serialization and the response envelope). One store, one
behavior, two transports — a save over HTTP and a save from the CLI write the
exact same markdown file and SQLite rows.

`serve` itself has no HTTP mapping — it *is* the server — and is listed as
`"transport":"cli-only"` in the route inventory (see
[`GET /commands`](#get-apiv1commands)) rather than silently omitted.

## Start a server

```bash
comemory serve --port 8787
```

prints the `/api/v1` base URL and a per-session token (with `--json`, a
one-line object carrying both). Everything below assumes `$TOKEN` holds that
token and the server is reachable at `$BASE` (e.g. `http://127.0.0.1:8787`).

## Auth

Every `/api/v1/*` request needs the session token, checked in this order
(`src/serve/router.rs::token_from_request`):

1. `X-Comemory-Token: <token>` header,
2. `Authorization: Bearer <token>` header (the console-api spec's form),
3. `?token=<token>` query parameter (the form a browser `EventSource` can
   send, since it cannot set custom headers),
4. a `comemory_token` cookie.

The server also rejects any request whose `Host` header does not name a
loopback host (DNS-rebinding defense) — this is not disableable, the
transport is loopback-only.

A missing/invalid token is `401`; a non-loopback `Host` is `403`. On
`/api/v1/*` both come back **enveloped JSON** (`code: "unauthorized"` /
`"forbidden"`, `meta.command: "auth"`); any other `/api/*` path (nothing is
mounted there any more) gets a plain-text body for the same failures.

```bash
curl -s -H "X-Comemory-Token: $TOKEN" "$BASE/api/v1/memories?limit=5"
```

### Repo scope

Every read that accepts a `repo` filter resolves a default when the request
omits it: the `X-Comemory-Repo: <label>` header first, then the server's own
`comemory serve --repo <label>`. An explicit `repo` parameter always wins
(`src/serve/scope.rs::RepoScope`). A client that sends neither sees no
change.

```bash
curl -s -H "X-Comemory-Token: $TOKEN" -H "X-Comemory-Repo: myrepo" "$BASE/api/v1/memories"
```

## The response envelope

Every `/api/v1/*` response — success and error alike — is one shape:

```json
{ "ok": true, "data": { "...": "..." }, "meta": { "command": "search", "elapsed_ms": 12 } }
```

```json
{
  "ok": false,
  "error": { "code": "not_found", "message": "memory not found: ab12cd34" },
  "meta": { "command": "delete", "elapsed_ms": 1 }
}
```

`data` comes in three families, matching the command it wraps:

- **paged** — the same `Page<T>` / search-envelope shape the CLI's `--json`
  mode already emits (`items`/`hits`, `limit`, `offset`, `total`,
  `has_more`), nested unchanged one level under `data`.
- **object** — a single result; `null` for commands with no output
  (`rebuild`, `ingest-code`).
- **job-accept** — `{ "job_id": "<16-hex>", "status": "queued" }` (see
  [Jobs](#jobs)).

One place owns the `Error → (HTTP status, code slug)` mapping —
`src/serve/envelope.rs::status_and_code` — and every `/api/v1` handler
answers through the envelope, so there is no second mapping to drift from
it. The current table:

| `Error` variant / condition | HTTP status | `code` |
|---|---|---|
| `NotFound` | 404 | `not_found` |
| `Forbidden` | 403 | `forbidden` |
| `BadRequest` | 400 | `bad_request` |
| `ConfirmationRequired` | 400 | `confirmation_required` |
| `Usage` | 400 | `usage` |
| `Config` | 400 | `config` |
| `Frontmatter` | 400 | `frontmatter` |
| `Document` | 400 | `document` |
| `Ast` (bad ast-grep pattern via `POST /code/ast`) | 400 | `ast` |
| `Json` (malformed payload, e.g. a bad vector) | 400 | `json` |
| `VecDimMismatch` | 422 | `vec_dim_mismatch` |
| `SchemaTooNew` (the binary is older than the on-disk schema) | 422 | `schema_mismatch` |
| `Unavailable` | 503 | `unavailable` |
| `Embedder` (embed command missing, failing, or timing out) | 503 | `embedder_unavailable` |
| `IndexRunning` (a queued/running `index-code` job holds the repo; `error.details = {repo, job_id}`) | 409 | `index_running` |
| `Unsupported` (a capability this build deliberately does not model) | 501 | `unsupported` |
| `Sqlite` with `SQLITE_BUSY` / `SQLITE_LOCKED` (retry with backoff) | 423 | `store_locked` |
| `Io` with `ErrorKind::NotFound` | 404 | `not_found` |
| write permit held by another mutating request/job (§Concurrency) | 503 | `busy` |
| `mutating` route on a `--read-only` server | 405 | `read_only` |
| missing session token / bad `Host` (router guard) | 401 / 403 | `unauthorized` / `forbidden` |
| anything else | 500 | `internal` |

The error object is `{code, message}`, plus a structured `details` member
for the variants that carry one (today: `index_running`).

One exemption, documented rather than papered over: axum's own `413` for an
over-limit body stays plain text (framework-level, before any handler runs).

## Route map

All paths below are relative to `/api/v1`. ○ = read (never `405 read_only`),
● = mutating. This table is generated by hand from
`src/serve/routes.rs::table()` and every resource's `table_entries()` —
the same data `GET /commands` (below) serves at runtime, so if the two ever
disagree, trust the running server.

**Memories** (`serve/routes/memories/`)

| Method + path | CLI command | Notes |
|---|---|---|
| ○ `GET /memories` | `list` | paged |
| ○ `GET /memories/{id}` | *(new)* | single-row lookup via `memory_meta`; `404` when absent/soft-deleted |
| ○ `GET\|POST /memories/search` | `search` | `GET` = no vector; `POST` = vector-capable |
| ○ `GET\|POST /context` | `context` | same GET/POST split |
| ● `POST /memories` | `save` | |
| ● `DELETE /memories/{id}?confirm=true` | `delete` | soft-delete, **confirm** |
| ● `POST /feedback` | `feedback` | |

**Code** (`serve/routes/code.rs`)

| Method + path | CLI command | Notes |
|---|---|---|
| ○ `GET\|POST /code/search` | `search-code` | no HTTP lazy-reindex (Non-Goal) |
| ○ `POST /code/ast` | `ast` | read, but `file` is containment-checked first |
| ● `POST /code/index` | `index-code` | **job**; `path` contained before the job is created |
| ● `POST /code/ingest` | `ingest-code` | **job**; NDJSON body, 64 MiB route-level limit |

**Sources** (`serve/routes/sources.rs`)

| Method + path | CLI command | Notes |
|---|---|---|
| ○ `GET /sources` | `sources` | `reconcile` forced `false` on a read-only server |
| ● `POST /sources` | `index` | **job**; every `path` entry contained first |
| ● `DELETE /sources?target=<id\|path>&confirm=true` | `unindex` | **confirm** |

**Graph** (`serve/routes/graph.rs`)

| Method + path | CLI command | Notes |
|---|---|---|
| ○ `GET /graph` | `graph` | reuses the legacy `build_code_graph`/`build_graph_page` pair |
| ○ `GET /edges` | `edges` | paged; `edge_fts` self-heal skipped on a read-only server |

**Learning** (`serve/routes/learning.rs`)

| Method + path | CLI command | Notes |
|---|---|---|
| ○ `POST /eval` | `eval` | **job**, read-class — no read-only gate, no confirm |
| ● `POST /tune` | `tune` | **job**; confirm only when `"apply":true` |
| ● `POST /bandit` | `bandit` | **job**; always mutating (upserts `bandit_arms`); confirm only when `"apply":true` |

**Maintenance** (`serve/routes/maint/`)

| Method + path | CLI command | Notes |
|---|---|---|
| ○ `GET /doctor` | `doctor` | |
| ○ `GET /consolidate` | `consolidate` | |
| ○ `GET /prune` | `prune` | dry-run report; `apply` forced `false` |
| ● `POST /prune` | `prune --apply` | **confirm** |
| ● `POST /gc` | `gc` | **confirm** |
| ● `POST /mine` | `mine` | not confirm-gated — a bounded scan, mutates only with `"apply":true` |
| ● `POST /hooks/install` | `install-hooks` | **confirm**; `repo` contained |
| ● `POST /rebuild` | `rebuild` | **job**, **confirm**; swaps the server's shared DB connection on success |

**Meta / jobs**

| Method + path | CLI command | Notes |
|---|---|---|
| ○ `GET /completions?shell=` | `completions` | script as a JSON string |
| ○ `GET /commands` | *(new)* | machine-readable route/command inventory |
| ○ `GET /jobs` | *(new)* | every retained job, newest first, paged |
| ○ `GET /jobs/{id}` | *(new)* | one job's record |
| ○ `GET /jobs/{id}/events` | *(new)* | SSE lifecycle stream |
| ○ `GET /health` | *(new)* | capability probe: `{read_only, version, embed_cmd_configured}` — no DB access |

**Console additions** (console-api spec, 2026-09-01 — every route is a
view over the same cores; ◇ = a job-creating route)

| Method + path | Notes |
|---|---|
| ○ `GET /overview`, `GET /overview/eval-series?limit=` | counters, index state, last index run, latest eval metrics, recall series, 4 recent memories |
| ○ `GET\|POST /search` | the console view over `find`: `q`, `scope` (`all\|memories\|code`), `kinds[]` (≤ 1), `limit`, `explain`; hits carry `type` and a derived `score_parts[]` explain strip |
| ○ `GET /search/suggest?q=` | mined expansions matching a query token + recent queries by prefix |
| ● `POST /search/{query_id}/feedback` | `{hit_id, type?, signal: used\|opened\|ignored, source?}` → the `feedback` core |
| ● `PATCH /memories/{id}` | frontmatter patch in place (same id); a `body`/`title` change mints a new id that `supersedes` the old |
| ● `POST /memories/{id}/restore`, `POST /trash/{id}/restore` | move the `.trash/` file back and re-mirror |
| ● `POST /memories/{id}/references/refresh` | re-pin anchored refs to the current HEAD, return the re-classified `code_refs` |
| ○ `GET /trash` | soft-deleted memories with `days_until_gc` |
| ○ `GET /graph/nodes?sort=pagerank\|path`, `GET /graph/nodes/{id}`, `GET /graph/nodes/{id}/neighbors?min_weight=`, `GET /graph/snapshot?edge_kinds=&min_weight=` | `{id}` is `file:<repo>:<path>` or `<repo>:<path>`, percent-encoded; the snapshot caps at 20 000 edges (`truncated`) |
| ●◇ `POST /graph/recompute` | job `graph-recompute`: PageRank re-projection + memory rank |
| ○ `GET /index/runs?repo=`, ●◇ `POST /index/runs` | history from `index_runs`; `{repo, path\|root, mode: incremental\|full}` → the `index-code` job, `409 index_running` while the repo has a live one, `400` when archived. `full` re-extracts every file and is lossy: it drops the repo's BYO `code_vec` rows and resets per-symbol access counters — re-run `ingest-code` afterwards |
| ○ `POST /jobs/{id}/cancel` | cooperative cancel (see Jobs). Read-class despite the `POST`: its route-table entry is `mutating: false`, because stopping a job writes nothing to the store — so it works on a `--read-only` server |
| ● `PUT /hooks/{name}?repo=` | `{enabled}`; `post_commit` and `post-commit` both accepted; `repo` is contained like `POST /hooks/install`'s (`403` outside every allowed root) |
| ●✓ `DELETE /sources/{target}?confirm=true` | path form of `DELETE /sources?target=` |
| ○ `GET /learning/summary`, `GET /learning/evals?limit=`, `GET /learning/golden-set?golden=`, `GET /learning/proposals`, `GET /learning/expansions` | learning-loop reads; `evals` rows carry derived `delta`/`is_baseline`/`is_best` |
| ◇ `POST /learning/evals` | alias of the `eval` job; `golden_set` alias, optional `knobs` override |
| ●✓ `POST /learning/proposals/{id}/apply`, ● `POST /learning/proposals/{id}/discard` | write the proposal's knobs into `config.toml` (and reload) / dismiss it |
| ○ `GET /config/retrieval`, ● `PUT /config/retrieval` | live ranking knobs with ranges; a partial update is validated before the file is touched (`400` on an out-of-range knob) |
| ○ `GET /doctor/system` | schema/backup/data-dir/embedder facts — never runs the embed command |
| ●◇✓ `POST /doctor/rebuild` | alias of the `rebuild` job (`scope` must be `all`) |
| ●◇ `POST /doctor/reembed` | `{target: memories\|code\|both}`; `503 embedder_unavailable` without an embed command. The embed command is probed once for its vector width before any row is written: an explicit leg that does not match its table's dim is `422 vec_dim_mismatch` with nothing written, while `both` re-embeds only the leg(s) that fit and names the rest in `skipped_legs` (neither fits → `400`) |
| ○ `GET /prune/candidates` | alias of `GET /prune`; `POST /prune` also takes `ids[]` and `dry_run` (the inverse of `apply`; it wins when both are sent, and a non-boolean value is `400`, never coerced) |
| ○ `GET /gc/policy`, ● `PUT /gc/policy`, ●◇✓ `POST /gc/run` | `trash_retention_days` / `telemetry_retention_days` / `last_run`; the job form of `gc` |
| ● `POST /repos`, ● `PATCH /repos/{name}`, ● `POST /repos/{name}/archive`, ●✓ `DELETE /repos/{name}` | connect a (contained) root, re-point its root, archive (stops indexing, keeps memories), disconnect (drops code rows, keeps memories) |
| ○ `GET /memory-stores`, ○ `GET /memory-stores/{id}`, ● `POST /memory-stores` (`501`), ● `PATCH /memory-stores/{id}`, ●◇ `POST /memory-stores/{id}/sync` | the one store (`default`): path, remote, `push_on_save`, git sync state; `PATCH` writes `[git] auto_sync` / `[git] remote` into `config.toml`, reloads, and answers from the reloaded config; the sync job commits `memories/` (pathspec-limited), pulls (`--rebase --autostash`; either conflict shape is reported by path, nothing is pushed), then pushes — `git push <remote> HEAD` when `[git] remote` is set, else a bare `git push` to the upstream |

### Request field mapping

Every clap arg id maps to the same-named snake_case JSON field on
`api::<cmd>::Request` (`--ref-file` → `ref_file`; a CSV flag like `--tags
a,b` becomes a real JSON array `["a", "b"]`). `--vector`/`--vector-stdin`
collapse to one `"vector": [f32, ...]` field. `GET` endpoints take query
params; `POST` endpoints take a JSON body; search-shaped endpoints
(`/memories/search`, `/context`, `/code/search`) accept both, because the
`GET` form cannot carry a vector.

A handful of CLI affordances are intentionally excluded from the HTTP
mapping (and from the parity test's field check): the global `--json` /
`--data-dir` flags, every `--vector-stdin` (the JSON body already carries
the vector inline), `index-code --extract` (streams JSONL to stdout, never
touches the DB), `graph --format` (HTTP is always JSON — `dot`/`html` stay
CLI-only), `search --only`/`search --path` (the interim document-search path
lives entirely in `cli::search_only`), stdin-body conveniences like `save -`
(the body is a required JSON field over HTTP), and clap's auto-injected
`help`/`version`.

**DELETE routes carry `?confirm=true` as a query parameter** (DELETE bodies
are unreliable across clients/proxies); POST routes carry `"confirm": true`
in the JSON body instead.

## Read-only mode

```bash
comemory serve --read-only
```

Every route flagged `mutating` in the table above returns `405`,
`code: "read_only"` — checked in `routes::guard_mutating` /
`routes::guard_job`, the one gate every mutating handler calls first.

Read routes keep working, including the `POST` bodies of
`/memories/search`, `/code/search`, `/context`, `/code/ast`, and `/eval`.
Three of them degrade a side effect rather than refusing outright:

- `search` / `search-code` / `context`: access tracking and
  `retrieval_log` writes are suppressed (`routes::track_for`); ranked
  results are unaffected.
- `sources`: `GET /sources` passes `reconcile: false` on a read-only
  server (list-only; the CLI always reconciles).
- `edges`: the one-time `edge_fts` self-heal is skipped.

## Confirm gate

Routes marked **confirm** above require explicit confirmation beyond the
session token alone — a token must never suffice to soft-delete a memory or
rebuild the database. POST routes need `"confirm": true` in the body;
`DELETE` routes need `?confirm=true` in the query string. Missing/false →
`400`, `code: "confirmation_required"`.

**Ordering** (`routes::require_confirm`'s doc comment, AC-19): a mutating,
confirm-gated route checks read-only **first**. On a `--read-only` server,
`POST /rebuild` without `confirm` still returns `405 read_only`, not `400
confirmation_required` — read-only outranks a missing confirm.

`tune` and `bandit` are a conditional case: both are always classified
`mutating` (the write-permit/read-only gate always applies — a report-only
run still burns real CPU), but the *confirm* check only fires when the
request body sets `"apply": true`. A report-only `tune`/`bandit` job needs
no confirmation.

## Path containment

Several routes accept a filesystem path directly (not a repo-relative
`file:<repo>:<path>` id): `index-code`'s `path`, `index`'s `path` entries,
`ast`'s `file`, `install-hooks`'s and `PUT /hooks/{name}`'s `repo`, and the `golden` file of
`eval`/`tune`/`bandit`. `serve::security::contain_abs(roots, p)`
canonicalizes `p` and requires it inside one of `roots`:

- nonexistent path → `400 bad_request`,
- outside every root (`..`-laden or symlink-escaping included) → `403
  forbidden`.

The allowed-roots set (`AppState::allowed_roots`) is the union of:

1. `--root <repo>=<path>` overrides,
2. every stored `repo_marker.root_path` row (`store::repo_marker_roots`),
3. the server process's own git work-tree root, when its cwd sits inside
   one (the bootstrap case for a fresh install with no `repo_marker` rows
   yet),
4. `--allow-path <dir>` entries (repeatable; see below).

Handlers enforce containment **before** calling into `api::<cmd>::run` —
the shared command core stays transport-agnostic and exactly as
unrestricted as the CLI (which already trusts the local filesystem).
Containment for a job-creating route runs before the job is even created,
so a rejected path never produces a `202`.

### `--root`

```bash
comemory serve --root myrepo=/abs/path/to/repo
```

Repeatable. Names a repo's working-tree root as `<repo>=<abs-path>`. Today
it does exactly two things:

1. **Containment allowlist.** The path (canonicalized) joins the
   allowed-roots set above, so a path-taking mutating route may touch files
   under it even when no `repo_marker.root_path` row names that root yet.
2. **Root resolution for `POST /memories/{id}/references/refresh`.** That
   route re-pins a memory's code references against the repo's HEAD, and an
   override wins over the stored `repo_marker.root_path` when it resolves
   the `<repo>` label to a checkout. A repo whose root resolves neither way
   is reported in the response's `skipped` list rather than failing the
   call.

It is **not** a general "resolve `<repo>` to a directory" switch: the read
routes (`GET /memories/{id}`'s reference freshness, `GET /repos`, the graph
routes) resolve through `repo_marker.root_path` alone, and `POST /code/ast`
takes an absolute `file` and only contains it. A repo indexed before the v7
schema captured its root gains a stored root the next time `index-code` (or
`POST /repos`) runs against it; `--root` is the stopgap for the refresh
route until then.

### `--allow-path`

```bash
comemory serve --allow-path /abs/path/to/golden-dir
```

Repeatable. Lets a mutating route (typically `eval`/`tune`/`bandit`'s
`golden` file) touch a path outside any indexed repo root. Each entry is
canonicalized at startup; an unusable entry (does not exist, unreadable)
fails server startup outright rather than being silently dropped.

## Jobs

`index-code`, `ingest-code`, `index`, `rebuild`, `eval`, `tune`, `bandit`,
`graph-recompute`, `reembed`, `gc` (the `POST /gc/run` form), and
`store-sync` run as background jobs: the `POST` returns immediately and the
work continues on a blocking-pool thread.

```json
HTTP/1.1 202 Accepted
Location: /api/v1/jobs/3f9a1c2b7e0d5a41

{ "ok": true, "data": { "job_id": "3f9a1c2b7e0d5a41", "status": "queued" }, "meta": {...} }
```

Poll status:

```bash
curl -s -H "X-Comemory-Token: $TOKEN" "$BASE/api/v1/jobs/3f9a1c2b7e0d5a41"
```

```json
{
  "ok": true,
  "data": {
    "job_id": "3f9a1c2b7e0d5a41",
    "command": "index-code",
    "status": "done",
    "started_at": "2026-08-06T12:00:00Z",
    "finished_at": "2026-08-06T12:00:04Z",
    "result": { "...": "the payload the synchronous form would have returned" },
    "error": null
  },
  "meta": {...}
}
```

`status` is one of `queued | running | done | error | cancelled`; `repo`
names the repo label a job works on (`null` for the others). `GET
/api/v1/jobs` lists every retained job, newest first, paged
(`?limit=&offset=`).

### Cancel: `POST /jobs/{id}/cancel`

Cooperative. A queued job becomes `cancelled` immediately (its body never
runs); a running job has its cancel flag set and stops at its next boundary
— `index-code` checks between files and rolls its one transaction back, so
nothing is half-written; `reembed` checks between rows. Every other job kind
cancels only while queued. `data` reports `{job_id, outcome: "cancelled" |
"requested"}`; an unknown id is `404`, a finished job `400`. Not
read-only-gated: stopping a job never writes to the store.

### SSE: `GET /jobs/{id}/events`

`text/event-stream` of the job's lifecycle. **Guarantee: current state on
connect, plus a guaranteed terminal event; intermediate transitions are
best-effort** — the underlying `tokio::sync::watch` channel keeps only the
latest value, so fast transitions can coalesce. The stream's first emission
is an explicit read of the current status (`borrow_and_update()`), so a
client that attaches *after* the job already finished still gets the
terminal event immediately — there is no "missed the event" race. The
handler ends the stream itself once a terminal event is emitted.

An unknown job id is `404 not_found` **before** the stream opens, on all
three job routes — a client never has to distinguish "no such job" from "a
job that never emits."

```bash
curl -N "$BASE/api/v1/jobs/3f9a1c2b7e0d5a41/events?token=$TOKEN"
```

```
event: running
data: {"job_id":"3f9a1c2b7e0d5a41","status":"running","result":null,"error":null}

event: done
data: {"job_id":"3f9a1c2b7e0d5a41","status":"done","result":{...},"error":null}
```

`?token=` auth (rather than the header) is what makes this reachable from a
browser `EventSource`, which cannot set custom headers.

Two additive event types interleave with the lifecycle events: `progress`
(`{job_id, done, total, unit}`, one per unit of work) and `log`
(`{job_id, line}`, one per log line — best-effort: a subscriber more than
256 lines behind loses the oldest; `GET /jobs/{id}` carries the durable
20-line `log_tail`). A client that only handles the lifecycle event names
sees exactly the sequence it saw before either existed.

### Write-permit FIFO

`index-code`'s repo walk (and similarly `rebuild`, `ingest-code`, a mutating
`tune`/`bandit`) holds SQLite's write lock for its whole run — not a brief
transaction. One process-wide write permit (a single-slot semaphore)
serializes **all** mutating work, jobs and synchronous requests alike:

- a mutating **job** awaits the permit and holds it for its full duration —
  a second `POST /code/index` while one is running just queues
  (`status: "queued"`) until the first releases it;
- a synchronous mutating **request** (`POST /memories`, `DELETE
  /memories/{id}`, …) `try_acquire`s instead — if the permit is held, it
  answers immediately with `503`, `code: "busy"`, and a `Retry-After: 5`
  header, rather than stalling into SQLite's own `busy_timeout` (and risking
  a save that writes markdown but not its SQLite mirror row);
- read requests never touch the permit.

This is a per-server-process guarantee. A concurrent **CLI** write from a
different process bypasses the permit and rides SQLite's own
`busy_timeout`, exactly as two CLI processes contending today.

### Job failure and the rebuild connection swap

A failed job reaches `status: "error"` with the same `{code, message}`
shape the synchronous envelope's `error` field carries (`GET /jobs/{id}`'s
`result` is `null`; the SSE stream ends with an `error` event).

`rebuild` is a special case: on success it renames a freshly built DB file
over the live one, then swaps the server's long-lived shared connection in
place (`AppState::swap_conn`) so later requests see the new DB rather than
the unlinked pre-rebuild inode. If that swap itself fails, the job still
reports `status: "error"` and the server is documented as serving stale
reads (the old inode) until restart — it never crashes. A successful
`tune`/`bandit --apply` job similarly reloads `config.toml` into
`AppState`'s swappable config slot, so HTTP ranking picks up the new blend
knobs without a restart.

## Body limits

The global request-body cap is 5 MiB (`router::BODY_LIMIT`, above axum's
2 MiB default so a large `POST /api/v1/memories` body reaches the handler).
`POST /api/v1/code/ingest` carries its own 64 MiB `DefaultBodyLimit` layer
for real NDJSON symbol batches (`application/x-ndjson`). An over-limit body
is axum's own framework `413` — plain text, not an envelope.

## `GET /api/v1/commands`

The machine-readable route inventory, derived at request time from clap
introspection (`Cli::command()`) so it cannot silently drift from the real
subcommand set:

```bash
curl -s -H "X-Comemory-Token: $TOKEN" "$BASE/api/v1/commands" | jq .
```

```json
{
  "commands": [
    { "name": "search", "transport": "http", "routes": ["GET|POST /api/v1/memories/search"] },
    { "name": "index-code", "transport": "http", "routes": ["POST /api/v1/code/index"] },
    { "name": "serve", "transport": "cli-only", "routes": [] }
  ]
}
```

## Quick start

Save a memory, search for it, then run a background reindex job and poll it
to completion:

```bash
comemory serve --port 8787 &
BASE="http://127.0.0.1:8787"
TOKEN="<paste the token printed at startup>"

# Save a memory over HTTP
curl -s -X POST "$BASE/api/v1/memories" \
  -H "X-Comemory-Token: $TOKEN" -H "Content-Type: application/json" \
  -d '{"body":"Postgres connection pool caps at 20 in prod","kind":"decision","tags":["db","postgres"]}'

# Find it again
curl -s -H "X-Comemory-Token: $TOKEN" \
  "$BASE/api/v1/memories/search?query=postgres%20connection%20pool"

# Kick off a code index job and poll it
JOB=$(curl -s -X POST "$BASE/api/v1/code/index" \
  -H "X-Comemory-Token: $TOKEN" -H "Content-Type: application/json" \
  -d '{"repo":"comemory","path":"/abs/path/to/comemory"}' | jq -r .data.job_id)
curl -s -H "X-Comemory-Token: $TOKEN" "$BASE/api/v1/jobs/$JOB"
```

## See also

- [Getting started](../getting-started.md) — the CLI loop the API mirrors.
- [CLI reference](../cli-reference.md) — every subcommand's flags, which
  `/api/v1` field-maps onto.
- [Architecture](../architecture.md) — storage layout and the retrieval
  pipeline both surfaces share.
- [Scenario catalog](../scenarios/README.md) — every command's `/api/v1`
  route and the `tests/serve_scenario_*.rs` journey that drives it.
