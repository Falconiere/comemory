# domains/projects/

**What belongs here:** engine-owned project management (epic #261), ported from
the comemory.io platform's `/v1/projects` contract: the charter and its limits,
slug derivation, the keyset cursor, the project activity writer, the body-free
change feed, and the command cores the CLI, the loopback HTTP server and the
MCP catalog all call.

**What does NOT belong here:** SQL (every project statement lives in
`store::{projects,project_read,project_activity,project_changes}`, the
declared tables in `store::schema_projects`), delivery (no file here imports `cli`, `serve` or
`mcp`), and the refusal vocabulary, which is `utilities::project_error`.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `activity.rs` | `record` | The one `project_activity_events` writer every mutation shares, called inside the mutation's own transaction with the admitted `Actor`, which also appends the event's `changed` feed row; never `activity_log`, which is command telemetry |
| `authority.rs` | `run` | The capability envelope every core runs under: the sealed `Command` trait, the `Actor` only `run` mints, the ported 27-verb table plus the engine's `ProjectChanges` reader, human tiers, agent capabilities, and the local operator and local agent defaults |
| `changes.rs` | `Request` | `project changes` / `GET /projects/changes` (`Verb::ProjectChanges`, a `project.read` reader): body-free frames `{seq, entity, project_id, event_id, op}` after a cursor, refusing a cursor past the head or below the oldest retained row; `record_deletion`, the `deleted` row #320 appends in its delete transaction |
| `charter.rs` | `validate` | A create request checked against every charter rule before any store access, in the platform's field order, and normalized for storage (lowercase id, canonical de-duplicated repositories, the lead) |
| `create.rs` | `Request` | `project create` / `POST /projects`: the draft charter, its repositories, its project-level criteria and one `project.created` event in one immediate transaction; the slug retried with `-2`, `-3`, … on a real collision |
| `keyset.rs` | `decode` | The platform's `<epochMillis>:<uuid>` keyset cursor over `(created_at, id)`: encode, and decode with a `400 invalid_request` for anything outside `^\d{1,15}:[0-9a-f-]{36}$`; later pages reuse it |
| `limits.rs` | `text` | The platform's charter and paging caps (`project-limits.ts`), counted in UTF-16 units, each breach a `422 invalid_request` naming field, reason and limit |
| `list.rs` | `Request` | `project list` / `GET /projects`: a keyset page newest first, filtered by status, health and `includeArchived` |
| `receipt.rs` | `run` | The idempotent-command runner every mutation goes through (#327): a `(principal, idempotencyKey)`-scoped `project_command_receipts` row written in the mutation's own immediate transaction; an exact replay returns the stored response and writes nothing, any other reuse is `409 idempotency_conflict` |
| `principal.rs` | `Principal` | The principal an envelope carries: `user` or `project_agent` plus an id; the `local-operator` and `local-agent` ids |
| `show.rs` | `Request` | `project show` / `GET /projects/{id}`: one charter; a malformed id is `400`, an unknown one `404 project_not_found` |
| `slug.rs` | `base_slug` | `projects.slug` from the charter name, and the `-2`, `-3`, … disambiguator |
| `timestamp.rs` | `iso` | Epoch milliseconds rendered as `toISOString()`, and `targetDate` parsing (calendar date = UTC midnight, RFC 3339 converted to UTC) |
| `view.rs` | `ProjectView` | The platform's charter view key for key minus `workspaceId`, and the batched load of a page's repositories and criteria |

## Rules this capability keeps

- **Ported, not redesigned.** Where the platform's shipped code and its design
  disagree, the code wins. The recorded divergences: a cap answers `422` (the
  platform's zod answers `400`), because the engine's vocabulary puts caps on
  the invariant edge; the view has no `workspaceId`, because one data
  directory is one workspace; `leadUserId` and `createdBy` carry principal ids.
- **One event per mutation, in its transaction.** A refused or failed command
  leaves no project row and no activity event.
- **No charter update verb.** After creation a charter changes only through an
  approved `project.update` proposal (#338).
- **One body-free feed row per committed mutation.** `activity::record`
  appends it, so a new mutation emits with no per-command code; a refused or
  rolled-back command emits none. The feed (`project_changes`) sits outside
  `PROJECT_TABLES` with no foreign key, so a deletion row outlives its
  project. Retention is unbounded: rows are ids only, and a cursor the feed
  cannot continue is refused, never answered with a page that skips.
- **Identities are client-generated UUIDs**, minted when absent, stored lowercase.
- **Repositories are shape-checked only** (canonical `owner/name`); the allow-list
  rule is #337's.

## Idempotency

Every mutation except hard deletion (#320) runs through `receipt::run`
(`receipt.rs`, #327), ported from the platform's
`project-command-receipt-service.ts`:

- **Key:** `idempotencyKey` is required on the core request, 1–200 UTF-16
  units; a breach is `422 invalid_request` (the recorded cap divergence). The
  CLI mints one when `--idempotency-key` is omitted, so only a named key is
  retry-safe.
- **Scope:** `(principal_type, principal_id, idempotency_key)` — not the
  command, not the project — so two principals may reuse a key.
- **Digest:** hex SHA-256 of canonical JSON `{"commandType", "body"}`; the
  body is the request minus `idempotencyKey` and bookkeeping (`workspaceId`).
  Create's body includes `id`, because the engine accepts a client id.
- **Replay:** same command type and digest → the stored core response,
  parsed back into the typed response. Nothing runs and nothing is written:
  no state change, no activity event, no `activity_log` row, and (once #324
  lands) no change notification.
- **Conflict:** another command type or digest under the key → `409
  idempotency_conflict`, before the command runs.
- **Failure:** the receipt is written in the command's transaction, so a
  failed command leaves none and its retry runs again.
- **Race:** the receipt is read inside the `BEGIN IMMEDIATE` transaction, so
  two processes racing one key serialize on SQLite's writer lock and the
  second replays the first.
- **Retention:** no TTL. A receipt lives until its project is hard-deleted,
  when `ON DELETE CASCADE` removes it, as on the platform.
- **Adapter independence:** the stored response is the core's, so a key
  first used through the CLI replays over HTTP with the same `data`.

A later mutation builds a `receipt::Keyed` from its key, command type and
digest body, does its writes inside `receipt::run`'s `apply`, and records
telemetry with `receipt::record`, which skips a replay.

## Authority

The platform decides authority and the engine only enforces it
(`authority.rs`, #315). Every core is a request type implementing the sealed
`Command` trait:

- `authority::run(ctx, &envelope, request)` checks the envelope first. A
  refusal returns before `execute`, so before the core touches the store:
  nothing is written, not even the `activity_log` row. Under the CLI's lazy
  context the database is never even opened; HTTP and MCP open their
  connection before the core runs.
- An admitted command gets the `Actor` it records on every row and project
  activity event.
- Only `run` mints an `Actor`, and only this capability can implement
  `Command`, so no core runs outside an envelope.

The project actor is never `utilities::activity::Origin.actor`, which is a
self-declared telemetry label.

- **Envelope:** a principal plus, for a `user`, a tier (`member` < `lead` <
  `owner`) or, for a `project_agent`, a capability set.
- **Capabilities** (ported unchanged): `project.read`, `proposal.create`,
  `work_packet.create`, `execution.update`, `evidence.create` and
  `health.update`. A list holding an unrecognised value grants nothing, and
  the warning never logs the value.
- **Verbs:** each names a rule, either *shared* (a tier for a human and a
  capability for an agent) or *human-only* (a tier; an agent is refused by
  principal kind).
  - `member` covers the readers, create, propose, work, evidence and
    work-item completion.
  - `lead` adds lifecycle, proposal review, health, and project completion
    and cancellation.
  - `owner` adds deletion.
- **Refusals:**
  - An agent is refused with `403 project_agent_scope`: `This command
    requires a signed-in human`, or `This grant does not carry the <cap>
    capability`.
  - A human below the tier is refused with `403 forbidden`, using the
    platform's sentence for the verb.
- **Local defaults, chosen by the adapter:**
  - The CLI is `Envelope::local_operator()`: `user` `local-operator` at
    `owner`.
  - MCP (`McpState::project_envelope`) and local-mode HTTP are
    `Envelope::local_agent()`: `project_agent` `local-agent` with all six
    capabilities.
  - #316 adds configuration, and #317 adds hosted mode's signed stamp.
- **Divergences from the platform:**
  - `project list` admits an agent with `project.read`, because the MCP
    budget makes `project_list` an agent reader.
  - An agent's create answers `403 project_agent_scope`, not `401`, because
    the engine has no session.

A later task names its verb from `Verb`, implements `Command` for its request,
and never re-decides who may run it.

## Command tree

Each later task adds its own group with its verbs; nothing is declared empty.

```text
comemory project
  create | show | list                      #326
  archive | restore | pause | resume        #328
  plan …                                    plan slice (#264)
  proposal …                                #335–#338
  item …          (ready, start, …)         #351
  execution …     (heartbeat, block, resume, request-review)  #352
  packet …                                  packets slice (#266)
  evidence …      (add, list)               #346
  approvals …                               approval inbox
  changes                                   #324 (read-only; HTTP for the relay)
```

## MCP budget

The curated catalog held 15 tools before this epic; projects add eight, and
human-only verbs get none:

- readers: `project_list`, `project_show` (this task — the catalog is 17);
- writers: `project_propose`, `project_work` (ready, start),
  `project_execution` (heartbeat, block, resume, request review),
  `project_work_packet`, `project_evidence`, `project_health`.

## One MCP tool, many verbs: the `action` rule

The parity model binds each catalog row to one clap command. A tool that takes
an `action` binds to a clap **group** instead:

- `ToolEntry` gains `actions: &'static [&'static str]`, empty for a leaf tool.
- The row's `command` is the group path (`project item`) and `actions` lists the
  leaves the tool exposes (`ready`, `start`). Human-only siblings in the same
  group stay unlisted.
- Parity resolves `<command> <action>` in the clap tree for every action and
  probes that leaf's argument ids against the tool request with `action` set.
- One tool name keeps one catalog row.

#351 implements this with `project_work`; #352 (`project_execution`) and #346
(`project_evidence`) follow it.
