# Real-time activity feed — Design

**Date:** 2026-09-20 **Status:** Approved **Author:** Falconiere R. Barbosa
**Topic:** One row per agent-visible command run, readable as a snapshot and streamed live over `/api/v1/activity`.

## Problem

`comemory` is driven mostly by coding agents, through three surfaces — the CLI,
the loopback HTTP API, and the MCP stdio server. Today nothing answers "what
are my agents doing right now": `retrieval_log` records tracked searches and
nothing else, `stats` and `overview` report the corpus rather than the traffic,
and a `save` an agent made over MCP is invisible until someone lists memories
and notices a new one.

The user wants to watch, live, what agents save, search, sync and give feedback
on — with enough per-call detail to recognize the call (which memory, which
query, which verdict), and with the caller identified when the caller told us
who it is.

## Non-Goals

1. **No `comemory activity` CLI command** in this iteration. The feed is an
   HTTP surface only; a CLI tail can follow once the shape has settled.
2. **No LLM token or cost accounting.** `comemory` never calls a model, so it
   has nothing to account for.
3. **No export off the machine.** Activity rows are local; they are not part of
   any sync payload and never reach `api.comemory.io`.
4. **No recording of commands outside the instrumented list** (§ Architecture).
   `completions`, `doctor`, `upgrade`, `install`, `stats`, `list`, `show`,
   `graph`, `eval`, `tune`, `rebuild` and friends stay unrecorded.
5. **No verbatim request or response bodies.** Only the bounded per-command
   summary in § Interfaces.
6. **No WebSocket transport and no server push to anything but a connected
   client.** SSE over the existing loopback server, nothing else.
7. **No user identity model.** `actor` is a self-declared label a caller
   supplied (MCP `clientInfo`, HTTP `User-Agent`, `COMEMORY_ACTOR`), never an
   authenticated principal.
8. **No `sync.push` / `sync.pull` commands of their own.** Neither is
   instrumented as a command: both need a live workspace to exercise, and a
   mocked one would prove nothing. A `sync pull` still shows up in the feed,
   because it applies what it fetched through `exchange::import` — so the feed
   reports what the pull *changed locally* (`sync.import`), which is the
   signal it exists for, on a path that is testable without a cloud.

## Architecture

**Record at the command core, not at the delivery surface.** Each instrumented
core is called exactly once per invocation by whichever surface was used, so
one call site serves CLI, HTTP and MCP and no row can be double-counted. A core
that another instrumented core calls internally is reached through an
uninstrumented inner entry, never through the public `run`: `memories::update`
re-saves through `save::run_with`, and `sync::exchange::import_rules` restores
through `restore::restore_one`, so a batch that already reports itself cannot
also report each entry. This
is the rule `retrieval::pipeline::log_retrieval` already follows for
`retrieval_log`, and the decisive trade-off against instrumenting
`cli.rs::run` + `serve::routes::respond` + `mcp::exec::run`: those three see a
rendered or generically-serialized result and could not produce a per-command
summary, and the CLI one would have to open a second connection after the
command closed its own — which would create `comemory.db` for commands that
never open it.

**Instrumented cores (v1), the verbs an agent uses:**

| Core | `command` |
|---|---|
| `domains::memories::save` | `save` |
| `domains::memories::delete` | `delete` |
| `domains::memories::update` | `update` |
| `domains::memories::restore` | `restore` |
| `domains::retrieval::search` | `search` |
| `domains::retrieval::find` | `find` |
| `domains::retrieval::context` | `context` |
| `domains::retrieval::search_code` | `search-code` |
| `domains::learning::feedback` | `feedback` |
| `domains::sync::exchange::import` | `sync.import` |
| `domains::code::index_code` | `index-code` |

`sync.push` / `sync.pull` get no command of their own (Non-Goal 8): they talk
to `api.comemory.io`, and this repository has no local cloud to run them
against, so a row they wrote could only be proven with a mock. `sync.import`
is the same exchange applied locally — by `POST /api/v1/sync/import` and by
`sync::pull`, which hands every fetched batch to that core — so a pull does
appear in the feed, describing what it changed on this machine, over a path
that is fully testable with real entries.

`command` values reuse the `RouteEntry::command` vocabulary so CLI, HTTP and
MCP cannot drift apart, and are declared as consts in
`utilities::activity::command` the way `utilities::telemetry::source` declares
the `retrieval_log` vocabulary — writers name a const, never an inline literal.

### Origin plumbing — the exact call sites

`Ctx` gains one field, `origin: Origin`, and `Ctx::borrowed` / `Ctx::lazy`
keep their signatures, defaulting it to
`{source: "cli", actor: env::COMEMORY_ACTOR}`. **No existing call site
changes.** Origin is then set by three additions, not by editing the 39
`Ctx::borrowed` sites that exist across `src/serve` and `src/mcp`:

1. `Ctx::with_origin(self, origin) -> Self` — a consuming builder.
2. **MCP: one site.** `mcp::exec::in_context` builds every tool call's `Ctx`;
   it appends `.with_origin(state.origin())`. `McpState` gains an
   `Arc<OnceLock<String>>` holding `"<clientInfo.name>/<version>"`, filled by
   overriding `ServerHandler::initialize` on `ComemoryServer`. That override
   **must** call `context.peer.set_peer_info(request.clone())` before
   delegating — rmcp 3.4's default `initialize` does it
   (`rmcp-3.4.0/src/handler/server.rs:323`) and an override that skips it
   leaves the peer without client info.
3. **HTTP: the instrumented routes only.** `routes::query_response` gains a
   sibling `query_response_with_origin(state, command, origin, query)`; the
   handlers for the twelve instrumented commands extract `HeaderMap`, build
   `Origin::http(user_agent)` (truncated to 120 chars), and call it. Every
   other route keeps `query_response` untouched. Routes whose core runs as a
   job (`index-code`) set the origin when the job closure builds its `Ctx`.

**Storage: a declared table**, `activity_log` in `src/store/schema_history.rs`
beside `eval_runs` / `gc_runs` / `index_failures`, with the migration generated
by `just migration activity_log`. An `AUTOINCREMENT` id gives the stream a
stable cursor (`id > after_id`), the pattern `index_failures` already uses; a
`desc(at)` index serves the newest-first snapshot.

**Recording never creates a database.** `record_in` returns before touching
`Ctx::conn` when the data dir has no `comemory.db` yet, so a command that
failed before opening the store (an `index-code` against a non-git path) keeps
the must-not-create-the-db invariant the repo already tests for.

**Writes are best-effort and take a connection, not a `Ctx`.**
`utilities::activity::record(conn, origin, …)` mirrors `log_retrieval`'s
signature so a core that already holds `ctx.conn()?` can call it without a
second mutable borrow. Any failure warns through `tracing` and returns — the
command's own result must never depend on telemetry.

**Reads: `domains::maintenance::activity`**, beside `stats` and `overview`,
called by both routes. It holds the must-not-create-the-db invariant: on a data
dir with no `comemory.db` it never calls `Ctx::conn` and answers with empty
items and empty rollups.

**Streaming: poll the table with a cursor — there is no channel.** The SSE
handler owns a cursor, selects rows with `id > cursor` every
`activity.stream_poll_ms` (default 500), emits one `activity` event per row in
id order, and advances the cursor. Rows are never dropped: the table is the
buffer, and a slow client simply reads further behind. The row *is* the wake
signal, so no wake file and no filesystem watch (the `sync.wake` pattern would
add an artifact for nothing when writer and reader share a database). Auth,
enveloping and heartbeat conventions are cloned from
`src/serve/routes/jobs.rs`; its `broadcast` channel is **not** — that exists
because job progress is in-process and unpersisted, which activity rows are not.

**Read-only servers do not record.** A `--read-only` server writes nothing to
the store, telemetry included; its own traffic is absent from the feed while
CLI and MCP processes keep recording. Both routes are `mutating: false` and
serve normally there.

**Retention belongs to `gc` and reuses the existing knob.**
`prune.learning_retention_days` (env `COMEMORY_LEARNING_RETENTION_DAYS`,
exposed on `GET|PUT /api/v1/gc/policy`) already governs `retrieval_log` and
`feedback_events` eviction; activity rows are the same class of telemetry and
use it rather than a second window. `maintenance::gc::Response` gains
`activity_rows: u64` beside `log_rows` / `event_rows`, and the `gc_runs`
declared table gains an `activity_rows` column — a second generated migration
in the same change.

## Interfaces / Schema

### Tables (`src/store/schema_history.rs`)

```rust
#[table(name = "activity_log")]
#[index("idx_activity_log_at", desc(at))]
pub struct ActivityLog {
    #[column(primary_key, autoincrement)] pub id: Integer,
    #[column(not_null)]                   pub at: Text,          // RFC3339 UTC
    #[column(not_null)]                   pub command: Text,     // utilities::activity::command const
    #[column(not_null, check = "source IN ('cli', 'http', 'mcp')")]
                                          pub source: Text,
    pub actor: Text,                                             // NULL when the caller declared none
    pub repo: Text,                                              // NULL when the call was unscoped
    #[column(not_null)]                   pub duration_ms: Integer,
    #[column(not_null, default = "1")]    pub ok: Integer,       // 0 on failure
    pub error_code: Text,                                        // utilities::error_code slug when ok = 0
    pub summary: Text,                                           // JSON object, NULL when summaries are off
}
```

Plus `GcRuns.activity_rows: Integer` (`not_null, default = "0"`).

### Writer (`src/utilities/activity.rs`)

```rust
pub struct Origin { pub source: &'static str, pub actor: Option<String> }
impl Origin { pub fn cli() -> Self; pub fn http(user_agent: Option<&str>) -> Self; pub fn mcp(client: Option<&str>) -> Self; }

pub mod source { pub const CLI: &str = "cli"; pub const HTTP: &str = "http"; pub const MCP: &str = "mcp"; }
pub mod command { pub const SAVE: &str = "save"; /* … one per instrumented core … */ }

/// Best-effort: warns and returns on any failure. Mirrors
/// `retrieval::pipeline::log_retrieval` — takes the connection the caller
/// already holds, never a `&mut Ctx`.
pub fn record(
    conn: &Connection,
    origin: &Origin,
    command: &'static str,
    elapsed: Duration,
    outcome: Result<&serde_json::Value, &Error>, // Ok(summary) | Err(command error)
    repo: Option<&str>,
);
```

### Summary shapes (JSON object, bounded)

| `command` | `summary` |
|---|---|
| `save` | `{id, title, kind, tags: n, supersedes: n}` |
| `delete` / `restore` | `{id, derived_stale}` |
| `update` | `{id, fields: ["body", "tags"]}` |
| `search` / `context` | `{query, hits, query_id, top: [id, …]}` |
| `find` | `{query, hits: {memory, code, document}, total, query_id, top: [id, …]}` |
| `search-code` | `{query, hits, lang}` |
| `feedback` | `{query_id, targets, used, irrelevant, used_code, irrelevant_code, provenance, known_query}` (`targets` capped at 5 ids) |
| `sync.import` | `{entries, applied, skipped, head_seq}` (counts derived from `ImportResponse.results`) |
| `index-code` | `{repo, files, mode}` |

`query` and `title` are bounded to 200 chars (`utilities::activity::bounded_text`);
`top` is capped at 5 ids.

### `GET /api/v1/activity`

Query: `repo`, `command`, `source`, `actor`, `since` (RFC3339), `limit`
(default 50, max 200), `offset`. Enveloped `{ok, data, meta}` like every other
route.

```json
{"items": [{"id": 412, "at": "2026-09-20T18:04:11Z", "command": "save",
            "source": "mcp", "actor": "claude-code/2.1.0", "repo": "comemory",
            "duration_ms": 37, "ok": true, "error_code": null,
            "summary": {"id": "a1b2c3d4", "title": "…", "kind": "decision", "tags": 3, "supersedes": 0}}],
 "total": 1284,
 "rollups": [{"command": "save", "runs": 12, "errors": 0, "p50_ms": 31, "p95_ms": 88}]}
```

### `GET /api/v1/activity/events`

Query: `after_id` (default: the newest row id at connect), plus the same
`repo`, `command`, `source`, `actor` filters. SSE events named `activity`, one
row object per event, `id:` set to the row id so a reconnecting `EventSource`
resumes via `Last-Event-ID`. A `: heartbeat` comment every 15s.

Route table entries: `{GET, "/activity", "activity", mutating: false}` and
`{GET, "/activity/events", "activity.events", mutating: false}` — synthetic
console names with no CLI counterpart, like `overview` and `learning.summary`.

### Config

| Key | Default | Meaning |
|---|---|---|
| `activity.enabled` | `true` | Record at all |
| `activity.summaries` | `true` | Write the `summary` column; `false` → NULL |
| `activity.stream_poll_ms` | `500` | SSE cursor poll interval |
| `prune.learning_retention_days` | existing | Reused for `gc` eviction; no new retention knob |

New env var: `COMEMORY_ACTOR` — the `actor` label a CLI run reports (`null`
when unset). Config section lives in `src/config/activity.rs`, following
`src/config/learning.rs`'s split-by-section layout.

## Failure modes and edge cases

| Case | Behavior |
|---|---|
| Activity insert fails (table missing, disk full, locked) | `tracing::warn!`, command returns its normal result; failure never propagates |
| Command core itself fails | Row written with `ok = 0` and the `error_code` slug; `summary` carries what was known (e.g. the query) |
| No `comemory.db` in the data dir | `GET /api/v1/activity` answers `items: []`, `total: 0`, `rollups: []`; no database is created |
| `--read-only` server | Both routes serve; that server's own commands write no rows |
| `activity.enabled = false` | No rows written; both routes still serve what is already stored |
| `activity.summaries = false` | Rows written with `summary = NULL`; no query text is persisted |
| Host sent no `clientInfo` | `actor = null`, `source = "mcp"` |
| SSE client slower than the writer | Nothing is dropped: the cursor advances only over rows actually sent, and unsent rows wait in the table. A client that falls behind reads older rows until it catches up |
| SSE client disconnects | The poll task ends with the response stream; no state is retained |
| SSE client reconnects | `Last-Event-ID` (or explicit `after_id`) resumes from the last delivered row id; rows evicted by `gc` in between are gone and the stream continues from the oldest surviving row above the cursor |
| Clock skew / format failure | `at` uses `memory_row::iso_format`; a format failure skips the row with a warn |
| Concurrent writers (CLI + MCP + serve) | Plain inserts under WAL and the existing busy timeout; no new locking |
| `activity_log` dropped from an existing db | `store::migrate` is keyed by `schema_meta` markers, so opening the db does not recreate it; inserts warn and commands still succeed |

## Acceptance criteria

- **AC-1:** A `save` through the MCP `save` tool, from a session whose host
  sent `clientInfo {name: "test-host", version: "1.0"}`, appends exactly one
  `activity_log` row with `command = "save"`, `source = "mcp"`,
  `actor = "test-host/1.0"`, and `summary.id` equal to the id the tool
  returned.
- **AC-2:** `comemory find "<query>" --json` in a terminal appends exactly one
  row with `source = "cli"`, `summary.query` equal to the query text and
  `summary.total` equal to the total hit count in the command's JSON output.
- **AC-3:** The same `search` issued over HTTP with
  `User-Agent: acme-console/3` appends one row with `source = "http"` and
  `actor = "acme-console/3"`.
- **AC-4:** A client connected to `GET /api/v1/activity/events?after_id=<n>`,
  where `<n>` is the newest row id at connect, receives — within a 10s test
  deadline — an `activity` event whose `id` is `> n` and whose payload is
  byte-equal to the item `GET /api/v1/activity?limit=1` returns for the row a
  *separate* `comemory save` process wrote after the connection opened.
- **AC-5:** `maintenance::activity::run` against a data dir with no
  `comemory.db` returns an empty page (`items: []`, `total: 0`, `rollups: []`,
  `cursor: 0`) and the data dir still contains no `comemory.db` afterwards. A
  fresh server's `GET /api/v1/activity` answers the same empty page (the
  server itself opens the database at startup, so the no-database half of the
  invariant is a core-level claim).
- **AC-6:** On a server started `--read-only`, an HTTP `search` returns `200`
  with its normal results and the `activity_log` row count is unchanged.
- **AC-7:** `GET /api/v1/activity?command=save&since=<ts>&limit=2` returns only
  `save` rows with `at >= ts`, newest first, at most 2 items, and a `total`
  equal to the number of matching rows.
- **AC-8:** With `activity_log` dropped from a real database, a `save` still
  exits 0 and its memory is readable by `comemory show` afterwards.
- **AC-9:** `comemory gc --json` on a store holding activity rows older than
  `prune.learning_retention_days` deletes exactly those rows, keeps the newer
  ones, and reports the deleted count as `activity_rows` in its JSON output.
- **AC-10:** With `activity.summaries = false` in `config.toml`, a `find`
  writes a row whose `summary` is NULL, and the query text appears in no
  `activity_log` column.
- **AC-11:** The `/api/v1` route table carries `activity` and
  `activity.events`, both with `mutating: false`, and the
  `docs/guides/http-api.md` route map lists them. (`GET /api/v1/commands` maps
  CLI subcommands onto routes; the feed has no CLI counterpart — Non-Goal 1 —
  so it is absent there by design, like `overview` and `health`.)
- **AC-12:** A `save` refused for a missing repo scope writes a row with
  `ok = 0` and the matching `error_code` slug, and no memory file is created.
- **AC-13:** `comemory feedback --query-id <id> --memory <id> --verdict used`
  against a real tracked query appends one row with `command = "feedback"` and
  `summary` carrying that target id, `target_kind = "memory"`, the verdict and
  `provenance = "manual"`.
- **AC-14:** `comemory index-code` over a real repository appends one row with
  `command = "index-code"` whose `summary.files` equals the number of distinct
  `code_symbols` paths the run wrote for that repo (the command prints nothing
  on a non-TTY success, so the store is the observable).
- **AC-15:** `POST /api/v1/sync/import` with a real two-entry batch appends one
  row with `command = "sync.import"` whose `summary.entries` is 2 and whose
  `summary.applied` equals the number of entries the response applied.

## Acceptance evidence

| AC | Real input | Expected observable | Boundary / failure | Runnable check |
|---|---|---|---|---|
| AC-1 | Real `comemory mcp` stdio session (the harness behind `tests/mcp__parity.rs`), real temp data dir | One row, fields as stated | Host with no `clientInfo` → `actor` NULL | `tests/mcp__activity.rs` |
| AC-2 | Real binary, memories saved first so `find` has hits | Row with query + total | Zero-hit query → `total: 0`, row still written | `tests/cli__activity.rs` |
| AC-3 | Real `serve` on a loopback port, HTTP client with a set `User-Agent` | Row with `source` / `actor` | Missing `User-Agent` → `actor` NULL | `tests/serve__routes__activity.rs` |
| AC-4 | Live SSE connection plus a second process running `comemory save`, 10s deadline | Event delivered, id above cursor, payload equal to the snapshot item | Reconnect with `Last-Event-ID` resumes above the cursor | `tests/serve__activity_stream.rs` (pattern: `tests/serve__jobs_progress.rs`) |
| AC-5 | Fresh empty `COMEMORY_DATA_DIR` | Empty payload; no db file created | — | `tests/serve__routes__activity.rs` |
| AC-6 | `serve --read-only` over a populated real store | Row count unchanged | — | `tests/serve__routes__activity.rs` |
| AC-7 | Real store with ≥3 rows across two commands and two timestamps | Filtered, ordered, counted | `since` newer than every row → empty, `total: 0` | `src/domains/maintenance/tests/activity.rs` |
| AC-8 | Real SQLite db with `DROP TABLE activity_log` executed | `save` exits 0; `show` finds the memory | — | `tests/cli__activity.rs` |
| AC-9 | Rows inserted with backdated `at` values | Old deleted, new kept, `activity_rows` reported | Retention window shorter than every row → all evicted | `tests/cli__gc.rs` |
| AC-10 | `activity.summaries = false` in a real `config.toml` | NULL summary; query text absent | — | `tests/cli__activity.rs` |
| AC-11 | The live route table and the rendered docs route map | Both entries present, `mutating: false` | — | `tests/serve__routes__activity.rs` |
| AC-12 | `save` without a resolvable repo scope | Row `ok = 0` with slug; no memory file | — | `tests/cli__activity.rs` |
| AC-13 | A real tracked query id from a prior `find`, then a real verdict | Row with target, kind, verdict, provenance | Verdict on an untracked id → command errors, row `ok = 0` | `tests/cli__activity.rs` |
| AC-14 | `comemory index-code` over a real two-file git repo | Row whose `files` equals the distinct indexed paths | A path that is not a git work tree → command errors, and no database is created | `tests/cli__activity.rs` |
| AC-15 | Real two-entry import batch over a loopback `serve` | Row with entries/applied/head_seq | Batch over the 500-entry cap → request rejected, row `ok = 0` | `tests/serve__routes__activity.rs` |

No mocks anywhere: every check drives the real binary, the real MCP stdio
server, or a real loopback `serve` against a real SQLite store in a temp data
dir.

## Documentation impact

- `docs/guides/http-api.md` — the two routes, their query parameters, the SSE
  event shape, and the read-only note.
- `docs/configuration.md` — the three `activity.*` keys and the
  `COMEMORY_ACTOR` env var.
- `docs/README.md` — index entry for this design under *Explanation*.
- `docs/architecture.md` — `activity_log` in the table inventory.
- `docs/guides/prune-and-gc.md` — `gc` now evicts activity rows under
  `prune.learning_retention_days` and reports `activity_rows`.
- `src/store/README.md`, `src/serve/routes/README.md`,
  `src/utilities/README.md`, `src/domains/maintenance/README.md`,
  `src/config/README.md` — per-file index entries for the new modules.
- `docs/scenarios/` — a scenario entry per route, matching the catalog layout.

## Open Questions

1. **CLI `actor` for agent-driven shell calls** — an agent running
   `comemory save` through a shell has no identity beyond `COMEMORY_ACTOR`.
   Owner: user. Non-blocking: `null` is honest, and the env var covers wrappers
   that care.
2. **`comemory activity` CLI tail** — Non-Goal 1 today. Owner: user.
   Non-blocking: adding it later is additive, but it would then need the two
   independent `CLI_ONLY` declarations the repo requires.
3. **Rollup window** — rollups cover the filtered result set. A console wanting
   "last hour" rollups independent of the item page needs an extra parameter.
   Owner: user. Non-blocking.

## Review

Reviewed 2026-09-20 against the `spec-review` checklist. One blocker (origin
plumbing hid 39 `Ctx::borrowed` call sites), six should-fixes (SSE channel
contradiction, duplicate retention knob, unreported `gc` count, writer borrow
signature, ambiguous `find` summary, `set_peer_info` obligation, untimed AC-4)
and four minors (wrong gate cited in AC-11, dropped-table behavior, test paths,
`COMEMORY_ACTOR` docs) were raised and are resolved above. **Status: Approved**
for planning.
