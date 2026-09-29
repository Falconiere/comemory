# The `/api/v1` HTTP API

**Goal:** drive every `comemory` subcommand over HTTP — from a local agent, an
editor extension, a console, or a script — through the loopback server
`comemory serve` starts, with no second copy of any command's logic.

## What it is

`comemory serve` is a loopback-only HTTP server whose whole surface is the
versioned REST API at `/api/v1` — there is no bundled web page. Almost
every CLI subcommand — `save`, `search`, `search-code`, `index-code`,
`eval`, `rebuild`, and 20-odd more — gets a `/api/v1` route. Both surfaces
share one command core under `src/domains/<capability>/`: each
`<capability>::<cmd>::run(&mut Ctx, Request)` holds a subcommand's logic once,
called by `cli::<cmd>::run` (which adds
arg-parsing and TTY/`--json` rendering) and by the matching HTTP handler
(which adds JSON (de)serialization and the response envelope). One store, one
behavior, two transports — a save over HTTP and a save from the CLI write the
exact same markdown file and SQLite rows.

Several subcommands have no HTTP mapping and are listed as
`"transport":"cli-only"` in the route inventory (see
[`GET /commands`](#get-apiv1commands)) rather than silently omitted: `serve`
itself — it *is* the server — `upgrade`, because a server must never
replace its own binary on request, and the platform cloud-sync verbs
`auth` / `workspaces` / `link` / `sync` (they talk to `api.comemory.io`,
not the local engine).

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
for the variants that carry one (`index_running`, `id_collision`, and every
project refusal below). Every error body — whichever constructor built
it — serializes its keys in the order shown:
`ok, error{code, message, details}, meta{command, elapsed_ms}`. A project
refusal's `details` keep the platform's key order; the other variants'
`details` are sorted.

### Project refusals

`Error::Project` carries the platform's twenty-two project codes
(`src/utilities/project_error.rs`). Each one's `error` member is the
platform's `/v1` `error` member byte for byte — same `message`, same
`details` keys in the same order — so the console parses an engine refusal
without change. The CLI exit code and the MCP channel come from the same
`classify` row.

| `code` | HTTP | exit | `error.details` after `code` |
|---|---|---|---|
| `project_not_found` | 404 | 64 | `projectId` |
| `work_item_not_found` | 404 | 64 | `workItemId` |
| `proposal_not_found` | 404 | 64 | `proposalId` |
| `execution_not_found` | 404 | 64 | `executionId` |
| `evidence_not_found` | 404 | 64 | `evidenceId` |
| `invalid_request` (schema edge, malformed cursor) | 400 | 64 | `{field, reason}` — no `code` member |
| `invalid_request` (invariant: entity, cap, cross-reference) | 422 | 65 | caller-supplied, no `code` member |
| `dependency_cycle` | 422 | 65 | `workItemIds` (cycle order) |
| `project_agent_scope` | 403 | 70 | — |
| `repo_not_allowed` | 403 | 70 | `repo` |
| `forbidden` (another actor's execution) | 403 | 70 | — |
| `proposal_stale` | 409 | 75 | `basePlanVersion`, `currentPlanVersion` |
| `proposal_already_reviewed` | 409 | 75 | — |
| `version_conflict` | 409 | 75 | `currentVersion` |
| `idempotency_conflict` | 409 | 75 | — |
| `invalid_transition` | 409 | 75 | — |
| `dependency_blocked` | 409 | 75 | `blockerWorkItemIds` |
| `completion_requirements_unmet` | 409 | 75 | `criterionIds`, `unmetCriterionIds`, `unverifiedCriterionIds`, `workItemIds` (all always present) |
| `evidence_unverified` | 409 | 75 | `criterionIds` |
| `execution_active` | 409 | 75 | — |
| `unauthorized` (hosted mode: missing or invalid principal stamp) | 401 | 70 | — |
| `context_unavailable` | 503 | 69 | — |
| `internal_error` (a project invariant guard fired) | 500 | 70 | `invariant` |

Over MCP every row is a tool-level error carrying `{code, message}` except
`internal_error`, which is a protocol error carrying only the code word.
`project_not_found` is one answer for an unknown project and one outside the
caller's reach: the body names only the supplied id, so it is never an
existence probe. No command raises these codes yet; each arrives with the
command that needs it (#323 onward).

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
| ○ `GET /memories` | `list` | paged; each row includes `author` (stable string, `""` when unset) |
| ○ `GET /memories/{id}` | *(new)* | single-row lookup via `memory_meta`; includes `author` (same empty-string contract); `404` when absent/soft-deleted |
| ○ `GET\|POST /memories/search` | `search` | `GET` = no vector; `POST` = vector-capable |
| ○ `GET\|POST /context` | `context` | same GET/POST split |
| ● `POST /memories` | `save` | content-addressed, idempotent replay; `created` in the response; `409 id_collision` — see [Save contract](#save-contract) |
| ● `DELETE /memories/{id}?confirm=true` | `delete` | soft-delete, **confirm** |
| ● `POST /feedback` | `feedback` | `{query_id, used[], irrelevant[], used_code[], irrelevant_code[], source?}`; memory and code verdicts commit atomically; `source` is `explicit` (default) or `implicit` and is stored as every verdict's `feedback_events.provenance` (`manual` / `implicit`) — any other value is `400 bad_request`; `data.provenance` echoes what was stored |

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

**Repos** (`serve/routes/repos.rs`)

| Method + path | CLI command | Notes |
|---|---|---|
| ○ `GET /repos` | `repos` | connected repo list; distinct from the `POST`/`PATCH`/`DELETE` admin verbs below (Console additions) |

**Stats** (`serve/routes/stats.rs`)

| Method + path | CLI command | Notes |
|---|---|---|
| ○ `GET /stats` | `stats` | store-wide counters |

**Find** (`serve/routes/find.rs`)

| Method + path | CLI command | Notes |
|---|---|---|
| ○ `GET\|POST /find` | `find` | `GET` = no vector; `POST` = vector-capable, same split as `/memories/search` |

**Hooks** (`serve/routes/hooks.rs`)

| Method + path | CLI command | Notes |
|---|---|---|
| ○ `GET /hooks` | `hooks` | list configured git hooks |
| ● `POST /hooks` | `hooks` | bulk set; see `PUT /hooks/{name}` (Console additions) for a single hook |

**Graph** (`serve/routes/graph.rs`)

| Method + path | CLI command | Notes |
|---|---|---|
| ○ `GET /graph` | `graph` | reuses the legacy `build_code_graph`/`build_graph_page` pair |
| ○ `GET /edges` | `edges` | paged; `edge_fts` self-heal skipped on a read-only server |

**Learning** (`serve/routes/learning.rs`)

| Method + path | CLI command | Notes |
|---|---|---|
| ○ `POST /eval` | `eval` | **job**, read-class — no read-only gate, no confirm |
| ○ `GET /eval/history` | `eval.history` | past eval runs |
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
| ● `POST /erase` | `erase` | **confirm** |
| ● `POST /rebuild` | `rebuild` | **job**, **confirm**; swaps the server's shared DB connection on success |

**Cloud sync** (`serve/routes/sync.rs`) — engine half of Slice 2, memories
and the code index; the platform Worker forwards `/v1/sync/*` here after
device auth, rate limits, and the authoritative org-repository policy. Distinct from git `POST /memory-stores/{id}/sync`.
CLI verbs `auth` / `workspaces` / `link` / `sync` are platform clients
(cli-only); see [Cloud sync](cloud-sync.md).

| Method + path | CLI command | Notes |
|---|---|---|
| ○ `GET /sync/changes?since=&limit=` | *(engine)* | append-only log page; empty → `{entries:[], next_seq:null, head_seq}` through the envelope |
| ● `POST /sync/import` | *(engine)* | batch apply (import rules 1–10); the managed Worker accepts top-level `repositories: {memoryId: canonicalOwnerName}` and strips it before forwarding. Per-entry `repo_not_allowed` is returned by the Worker policy gate |
| ○ `GET /sync/manifest` | *(engine)* | 256 bucket hashes over live content hashes + `head_seq` |
| ○ `GET /sync/replica/changes?since=&limit=&kind=&epoch=` | *(engine)* | `replica-v1` page above a cursor: each entry carries the payload accepted at that position and a `payload_state` (`present`/`absent`/`erased`/`expired` — the last for a shared feedback or activity event past retention, whose digest still deduplicates a later offer). A foreign `epoch` is `409 epoch_mismatch` and a `since` past the head is `409 cursor_ahead`, never an empty page |
| ○ `GET /sync/replica/manifest` | *(engine)* | stream epoch, head, `capabilities` (empty until journal seeding completes), per-kind live count + 256 bucket digests, `bootstrap {state, seeded}`, and `needs_embedding` (memories stored without a usable vector) |
| ○ `GET /sync/replica/events?since=&epoch=` | *(engine)* | notification-only frames `{sequence, entity_kind}` — resumable from any position, and carrying no content |
| ● `POST /sync/replica/import` | *(engine)* | apply a `replica-v1` envelope (≤500 operations, ≤5 MiB): per-operation `accepted`/`duplicate`/`rejected_*`/`payload_erased` with the assigned `sequence`. A body carrying `workspace_id` is `400` — scope comes from the credential. An operation may carry `vector {model, dims, f32}` alongside its payload; it is **excluded from `payload_digest`**, so an embedding never changes a revision's identity. An operation for an entity this engine still owes an unpushed change to is `rejected_stale` — the pending payload is the only record of that edit |
| ● `POST /sync/replica/stage` | *(engine)* | one part of an oversized revision (`staging_id`, `part_index`, `part_count`, `bytes`); invisible to `changes`/`manifest` until activated |
| ● `POST /sync/replica/activate` | *(engine)* | assemble a staged upload and accept it as one operation; a missing part is `409 staging_incomplete` |
| | | Three `entity_kind`s are carried today: `memory`, `code_generation` and `document_revision`. A code generation's `entity_key` is the repo label and its payload is the whole projection (manifest, snippet-free symbols, edges) — accepted as one unit or not at all. One planned against a parent the repo has moved past is `rejected_stale` — a per-operation disposition, so the rest of the envelope still applies — never merged; a `tombstone` for it means the repository is authoritatively empty, never that the sender lost its checkout. A document revision's `entity_key` is its `shared_id` — the digest of the canonical repository and the document's repository-relative path — and its payload carries the extracted passages and links, never the file. One whose key, id, passage ordinals or links do not hold together is `rejected_invalid` before anything is written; a `tombstone` forgets that one document and touches no local row |
| ○ `GET /sync/code/manifest?repo=` | *(engine)* | the code-index half: `{repo, head, mined_commit, files:[{path, blob_oid}]}` — what the workspace holds for one label, empty for an unknown one |
| ● `POST /sync/code/import` | *(engine)* | one batch of a repo's snippet-free projection (≤500 files): `{repo, head?, mined_commit?, files:[{path, blob_oid, symbols, imports}], removed:[path], cochange?:[{from,to,weight}]}` → `{applied, removed, rejected:[{path, reason}], head}`. Any rejection refuses the whole batch; rank is recomputed at the tail |

Platform device keys, rate limits (`sync_pull` 120/min, `sync_import` 30/min,
`sync_workspace` 600/min), and repository-policy source of truth live in comemory.io
`docs/platform-api.md` § Slice 2.

Managed `/v1/sync/*` data requests carry
`X-Comemory-Sync-Protocol: repository-policy-v1` and
`X-Comemory-Policy-Revision: <integer>`; responses echo both. The status route
negotiates the workspace id, allowlist, confirmed label mappings, revision,
protocol and import gate. The engine-only loopback routes above do not enforce
these platform headers.

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
| ○ `GET /activity?command=&source=&actor=&repo=&since=&limit=&offset=` | the recorded-command feed: one row per instrumented run (`save`, `delete`, `update`, `restore`, `search`, `find`, `context`, `search-code`, `feedback`, `sync.import`, `index-code`, `project.create`) with its bounded summary and `device` (`null` for a run recorded here, the origin device for one imported from another machine), plus per-command `rollups` and the `cursor` its stream starts from. `limit` defaults to 50, caps at 200 |
| ○ `GET /activity/events?after_id=&command=&source=&actor=&repo=` | the same feed as SSE: one `activity` event per row, the event `id` being the row id. Starts at the newest row unless `after_id` or `Last-Event-ID` says otherwise; `?token=` works here, as an `EventSource` cannot set headers |
| ○ `GET\|POST /search` | the console view over `find`: `q`, `scope` (`all\|memories\|code`), `kinds[]` (≤ 1), `limit`, `explain`; hits carry `type` and a derived `score_parts[]` explain strip |
| ○ `GET /search/suggest?q=` | mined expansions matching a query token + recent queries by prefix |
| ● `POST /search/{query_id}/feedback` | `{hit_id, type?, signal: used\|opened\|ignored, source?}` → the `feedback` core; `source: explicit\|implicit` is stored as the verdict's provenance (`manual` / `implicit`), on `ignored` as much as on `used` |
| ● `PATCH /memories/{id}` | frontmatter patch in place (same id); a `body`/`title` change mints a new id that `supersedes` the old |
| ● `POST /memories/{id}/restore`, `POST /trash/{id}/restore` | move the `.trash/` file back and re-mirror |
| ● `POST /memories/{id}/references/refresh` | re-pin anchored refs to the current HEAD, return the re-classified `code_refs` |
| ○ `GET /trash` | soft-deleted memories with `days_until_gc` |
| ○ `GET /graph/nodes?sort=pagerank\|path`, `GET /graph/nodes/{id}`, `GET /graph/nodes/{id}/neighbors?min_weight=`, `GET /graph/snapshot?edge_kinds=&min_weight=` | `{id}` is `file:<repo>:<path>` or `<repo>:<path>`, percent-encoded; the snapshot caps at 20 000 edges (`truncated`) |
| ○ `GET /graph/nodes/{id}/source` | Reads up to 1 MiB of UTF-8 source from the indexed local worktree, contained to that repo root. Returns `{content, reason}`; `content: null` has reason `no_local_worktree`, `worktree_missing`, `file_missing`, `not_a_file`, `file_too_large`, or `file_unreadable`. Synced cloud projections have no local worktree. |
| ●◇ `POST /graph/recompute` | job `graph-recompute`: PageRank re-projection + memory rank |
| ○ `GET /index/runs?repo=`, ●◇ `POST /index/runs` | history from `index_runs`; `{repo, path\|root, mode: incremental\|full}` → the `index-code` job, `409 index_running` while the repo has a live one, `400` when archived. `full` re-extracts every file and is lossy: it drops the repo's BYO `code_vec` rows and resets per-symbol access counters — re-run `ingest-code` afterwards |
| ○ `POST /jobs/{id}/cancel` | cooperative cancel (see Jobs). Read-class despite the `POST`: its route-table entry is `mutating: false`, because stopping a job writes nothing to the store — so it works on a `--read-only` server |
| ● `PUT /hooks/{name}?repo=` | `{enabled}`; `post_commit` and `post-commit` both accepted; `repo` is contained like `POST /hooks/install`'s (`403` outside every allowed root) |
| ●✓ `DELETE /sources/{target}?confirm=true` | path form of `DELETE /sources?target=` |
| ○ `GET /learning/summary`, `GET /learning/evals?limit=`, `GET /learning/golden-set?golden=`, `GET /learning/proposals`, `GET /learning/expansions`, `GET /learning/recall-status` | learning-loop reads; `summary.implicit_share` is the share of `feedback_events` whose provenance is not `manual` — the `auto_*` rewards and every route-written `source: implicit` verdict; `evals` rows carry derived `delta`/`is_baseline`/`is_best` |
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

**Projects** (`serve/routes/projects.rs`, #326 — the platform's `/v1/projects`
paths, so a hosted cutover forwards without remapping; see
[the projects scenario](../scenarios/project.md))

| Method + path | CLI command | Notes |
|---|---|---|
| ● `POST /projects` | `project create` | **`403 project_agent_scope`** in local mode (below); for an admitted human, **`201`**. Body is the platform's: `idempotencyKey` (required, 1–200 UTF-16 units; `400` when absent, `422` outside the range), `name`, `keyPrefix`, `outcome`, optional `id` (client UUID, minted when absent), `successCriteria[]`, `constraints[]`, `nonGoals[]`, `repositories[]` (canonical `owner/name`), `leadUserId` (defaults to the caller's principal id), `targetDate` (`YYYY-MM-DD` = UTC midnight, or RFC 3339); `workspaceId` is accepted and ignored. Writes the charter and one `project.created` activity event in one transaction. Every limit answers `422 invalid_request` with `{field, reason, limit}`; a duplicate `keyPrefix` is `422 {field: keyPrefix, reason: duplicate}`. Idempotent: the same key and body replays the stored answer (`201`, no write); the same key with another body is `409 idempotency_conflict` |
| ○ `GET /projects?status=&health=&includeArchived=&cursor=&limit=` | `project list` | `{projects, nextCursor}`, newest first by `(createdAt, id)`; `limit` 1–100 (default 20, `422` past it); `cursor` is the previous page's `<epochMillis>:<uuid>` — a malformed one is `400 invalid_request`; archived projects only with `includeArchived=true`; a repeated or unknown parameter is `400` |
| ○ `GET /projects/{id}` | `project show` | `{project}`; a non-UUID id is `400`, an unknown one `404 project_not_found` |
| ○ `GET /projects/{id}/plan` | `project plan show` | The committed plan (#335), the platform's `ProjectPlanResponse`: `{plan: {projectId, planVersion, milestones, workItems, criteria, dependencies}}` under `Verb::PlanRead` (`project.read`). Before the first approval `planVersion` is `0` with no milestones, work items or dependencies; `criteria` then holds only the charter's project-level success criteria. Milestones by `(position, id)`, work items by `(position, number)`, criteria of both levels by `(position, id)` (`workItemId` null for a project-level one), and `blocks` edges `{blockerId, blockedId}`. Archived milestones, items and criteria are absent, and so is every edge naming an archived item; archiving a milestone or item archives nothing under it, so, as on the platform, a live item can name an archived `milestoneId` and a live criterion an archived `workItemId`, and both are kept. A non-UUID id is `400`, an unknown one `404 project_not_found`, an agent without `project.read` `403 project_agent_scope` |
| ○ `GET /projects/{id}/activity?order=&cursor=&limit=` | `project activity` | One project's append-only activity log (#331), a `project.read` reader (`Verb::ActivityRead`): `{events, nextCursor}`, each event `{id, projectId, actorPrincipalType, actorPrincipalId, eventType, entityType, entityId, payload, createdAt}`. `order` is `desc` (newest first, the default) or `asc`; both walk `(createdAt, id)` with ties broken by `id`, so a walk under concurrent writes returns every earlier event exactly once. `cursor` is the previous page's `<epochMillis>:<uuid>`; `limit` 1–200 (default 50, `422` past it). A malformed id, cursor or order, a non-integer `limit`, or a repeated or unknown parameter is `400`; an unknown project `404 project_not_found`. MCP reads the same page as `project_show` with `view: "activity"`, and refuses the page fields on any other view |
| ○ `GET /projects/changes?after=&limit=` | `project changes` | The body-free change feed (#324): a `project.read` reader under the caller's envelope (`Verb::ProjectChanges`), so the local agent every local-mode caller runs as reads it; `data` is the bare frame array, the `/sync/replica/events` shape — `[{seq, entity: "project", project_id, event_id, op}]` ascending by `seq`, one frame per committed mutation (`op: changed`, `event_id` = its activity event) and one per hard deletion (`op: deleted`, `event_id` = the project id). No charter, proposal, work-item, evidence or activity body ever appears. `after` defaults to `0`, `limit` to `100` (1–1000, `422` past it). Retention is unbounded; a cursor past the head is `422 {field: after, reason: cursor_ahead}` and one below the oldest retained row `422 {field: after, reason: cursor_expired}` — never a page that skips. A non-integer or negative `after`, or a repeated or unknown parameter, is `400`. The static segment wins over `{id}` |

A malformed project body answers the platform's schema-edge shape rather
than axum's plain-text rejection: `body must be an object`, `<field> is
required`, or `<field> is invalid`, with `{field, reason}` details
(`successCriteria.3` for an array element).

Every project route runs under a capability envelope (#315), and a local-mode
server gives every caller the same one: the **local agent**, a
`project_agent` (`local-agent`) holding all six capabilities (`project.read`,
`proposal.create`, `work_packet.create`, `execution.update`,
`evidence.create`, `health.update`) and, by principal kind, no human verb. No
request header changes it. The two `GET` routes therefore answer, while
`POST /projects` — a human-only verb — answers `403 project_agent_scope`
(`This command requires a signed-in human`) and writes nothing, not even for
an over-limit charter: authority is checked before validation, after the body
parses (`400`) and after the read-only gate (`405`). Charter projects through
the CLI, which runs as the local operator; #316 adds the setting that makes a
local-mode server act as a human, and #317 the hosted mode's signed stamp.

Project mutations are idempotent (#327): `POST /projects` today, and each
later mutation as it lands, except hard deletion (#320). The
`idempotencyKey` is scoped to the caller's principal, not to the command or
the project, so two principals may reuse one key. The first run stores its
response as a command receipt in the same transaction as the change. A retry
with the same key and the same body returns that response and writes nothing,
not even a `project_changes` frame. The digest covers the command type and
every body field exactly as sent, except `idempotencyKey` and `workspaceId`,
so a retry must resend the same bytes: `Falconiere/comemory` and
`falconiere/comemory` are different bodies. Any other reuse answers `409
idempotency_conflict`. A failed command stores no receipt, so its retry runs
again. Receipts live until their project
is hard-deleted; there is no TTL. The receipt holds the core response, so a
key first used through the CLI replays over HTTP with the same `data`.

### Request field mapping

Every clap arg id maps to the same-named snake_case JSON field on the
command core's `Request` — `domains::<capability>::<cmd>::Request`
(`--ref-file` → `ref_file`; a CSV flag like `--tags
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

### Save contract

`POST /memories` (and `comemory save`) is **content-addressed**: the memory
id is the first 8 hex characters of `SHA-256(body.trim_end())`, computed
before anything is written. That makes the save idempotent without any
idempotency key — the body itself is the key, and a caller can reproduce
it on a retry, which a server-minted token could never guarantee. The
promise, with the tests that pin it (`tests/cli__save_3.rs`,
`tests/serve__routes__memories__write.rs`):

- **A replay creates no second memory.** A body byte-identical (after
  `trim_end`) to an existing memory lands on the same id and the same
  markdown file. The response reports which happened:

  ```json
  { "ok": true, "data": { "id": "faa80a60", "path": "/…/faa80a60-….md", "created": true } }
  { "ok": true, "data": { "id": "faa80a60", "path": "/…/faa80a60-….md", "created": false } }
  ```

  `created: true` means no memory with that id existed — live or trashed —
  before this call; `created: false` means the call overwrote one. A
  downstream that stores the id as a durable back-link can tell "I
  repaired a lost link" from "the save had never happened". `duplicate_of`
  is a different signal (a SimHash *near*-duplicate of another memory) and
  keeps its optional slot.
- **Identity is the body alone.** `kind`, `repo`, `tags`, `author` and
  `quality` are overwritten last-writer-wins on a replay; the
  `updated_at` row column moves, `created_at` and the markdown `created`
  do not.
- **A trashed id is revived.** Replaying the body of a soft-deleted
  memory moves it back out of `.trash/`, clears `deleted_at`, keeps its
  original `created`, and answers `created: false`. The same holds for a
  memory a later `PATCH /memories/{id}` superseded: it comes back live,
  still annotated `superseded_by`.
- **A collision is refused, never absorbed.** The id is 32 bits, so two
  different bodies *can* share one (birthday bound: about 1 % odds at
  ~9 000 memories in a store, 50 % at ~77 000). A save whose id matches an
  existing memory — live or trashed — whose `content_hash` differs is
  refused **before any write** with `409 {"code": "id_collision",
  "details": {"id": "0adf80f7"}}` (CLI: exit 65). Change the body to save
  it. The id is therefore unique per store by construction; no widening
  is planned.
- **Concurrency.** Over HTTP the write permit serializes saves, so the
  second of two concurrent replays sees the first. Two CLI processes have
  no shared lock and may both answer `created: true`; the atomic rename
  and the row upsert still leave exactly one file and one row.

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
- `activity`: the server records no rows of its own — read-only means no
  store write, telemetry included. Both feed routes keep serving what is
  already stored, and CLI and MCP processes on the same data dir keep
  recording.

Separately, wherever tracking IS on, a request for a page past the head of the
ranking (`offset > 0`) still records its `retrieval_log` row — so `POST
/feedback` can still cite that `query_id` — but bumps no access counts, so a
deep page cannot reinforce itself. See §5.1 of `docs/architecture.md`.

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

Handlers enforce containment **before** calling into
`domains::<capability>::<cmd>::run` —
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

### Job failure and rebuild

A failed job reaches `status: "error"` with the same `{code, message}`
shape the synchronous envelope's `error` field carries (`GET /jobs/{id}`'s
`result` is `null`; the SSE stream ends with an `error` event).

`rebuild` is a special case: it holds `memory-save.lock` (bounded by
`[sync] pause_wait`, `503 busy` past it) while it builds a fresh database and
replaces the server's long-lived shared connection's content in place
through SQLite's online backup API (`store::replace_in_place`), so later
requests see the new content with no reconnect and no unlinked inode. A
successful `tune`/`bandit --apply` job similarly reloads `config.toml` into
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
    { "name": "serve", "transport": "cli-only", "routes": [] },
    { "name": "upgrade", "transport": "cli-only", "routes": [] }
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
