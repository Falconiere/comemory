# domains/projects/

**What belongs here:** engine-owned project management (epic #261), ported from
the comemory.io platform's `/v1/projects` contract: the charter and its limits,
slug derivation, the keyset cursor, the project activity writer and its
keyset page, the body-free change feed, the transfer bundle (#342), and the
command cores the CLI, the loopback HTTP server and the MCP catalog all call.

**What does NOT belong here:** SQL (every project statement lives in
`store::{projects,project_read,project_plan,project_proposals,project_activity,project_changes,project_evidence,project_transfer,project_transfer_write,project_binding}`,
the declared tables and their transfer classes in `store::schema_projects`, the
carried-table shapes in `store::project_table_shape`), delivery (no file here imports `cli`, `serve` or
`mcp`), and the refusal vocabulary, which is `utilities::project_error`.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `activity.rs` | `record` | The one `project_activity_events` writer every mutation shares, called inside the mutation's own transaction with the admitted `Actor`, which also appends the event's `changed` feed row; never `activity_log`, which is command telemetry. Returns a `#[must_use]` `Recorded` whose `local_only` is set when the project is bound by a transfer |
| `activity_page.rs` | `Request` | `project activity` / `GET /projects/{id}/activity` (`Verb::ActivityRead`, a `project.read` reader, #331): one project's events as a keyset page over `(created_at, id)`, `order` `desc` (default) or `asc`, ties broken by id, `limit` 1–200 (default 50); an unknown project is `404 project_not_found`. It also holds `EventView` (the platform's event view key for key, a non-object payload read as `{}`) and `split` (the `limit + 1` page arithmetic), both shared with the cross-project feed (#332) |
| `authority.rs` | `run` | The capability envelope every core runs under: the sealed `Command` trait, the `Actor` only `run` mints, the ported 27-verb table plus the engine's `ProjectChanges` reader and the two transfer verbs, human tiers, agent capabilities, and the local operator and local agent defaults |
| `binding.rs` | `record` | A project's transfer binding (direction, remote, effective digest, time, the actor a remap replaced): recorded by an import (and by #345 after an upload) and shown by `project show` |
| `bundle.rs` | `Bundle` | The transfer bundle: format v1, the canonical form (carried tables parents first, rows sorted by primary key in Rust), its SHA-256 digest, a header-first `parse`, and the `ActorRemap` that rewrites matching principal pairs |
| `bundle_check.rs` | `check` | A parsed bundle checked against the carried tables before any store access: `schema_mismatch` for another engine's table or column list, `malformed` for a bad row — including any reference to a row the bundle does not hold |
| `changes.rs` | `Request` | `project changes` / `GET /projects/changes` (`Verb::ProjectChanges`, a `project.read` reader): body-free frames `{seq, entity, project_id, event_id, op}` after a cursor, refusing a cursor past the head or below the oldest retained row; `record_deletion`, the `deleted` row #320 appends in its delete transaction |
| `charter.rs` | `validate` | A create request checked against every charter rule before any store access, in the platform's field order, and normalized for storage (lowercase id, canonical de-duplicated repositories, the lead) |
| `create.rs` | `Request` | `project create` / `POST /projects`: the draft charter, its repositories, its project-level criteria and one `project.created` event in one immediate transaction; the slug retried with `-2`, `-3`, … on a real collision |
| `evidence.rs` | `EvidenceView` | Typed evidence shared by attach and page (#346): the closed `KINDS` and `TRUSTS`, `read_trust` (a stored value outside `TRUSTS` reads as `invalid`) and `trust_filter` (`invalid` also keeps those values), `Claim::check_shape` (the platform's claim shape, `commitSha` hex 1–256) and `Claim::initial` (the trust matrix), `StoredMetadata` (`{repo, commitSha, reason, claim, provider}`, ≤ 16 KiB encoded) and the platform's view key for key |
| `evidence_add.rs` | `Request` | `project evidence add` / `POST /projects/{id}/evidence` / MCP `project_evidence` (`Verb::EvidenceCreate`): under the receipt, one transaction checks the project, the repository gate (`403 repo_not_allowed`), the criteria (`422 … reason: unknown`) and the work item (`404 work_item_not_found`), then writes the row, its criterion links and one `project.evidence.recorded` event |
| `evidence_check.rs` | `validate` | The attach's checks before any store access, in the platform's schema order: UUIDs and the kind vocabulary (`400`), the text caps (`422` with the limit), the absolute URL, the canonical `repo`, the hex `commitSha` and at most 20 criterion ids |
| `evidence_page.rs` | `Request` | `project evidence list` / `GET /projects/{id}/evidence` / `project_show` with `view: "evidence"` (`Verb::EvidenceRead`): a keyset page newest first over `(created_at, id)`, `kind`, `trust` and `workItemId` alone or together, `limit` 1–100 (default 50); an unknown project `404`, an unknown item filter an empty page |
| `export.rs` | `Request` | `project export`: one project's carried rows as a canonical bundle, read in one transaction; `snapshot` is the same read inside a caller's transaction. Human-only at `member` |
| `import.rs` | `Import` | `project import`: check, verify the digest, remap, then in one immediate transaction answer `unchanged`/`skipped` for an existing id or refuse a taken key prefix or slug, write the rows under deferred keys and record the binding; no activity event. Human-only at `owner` |
| `keyset.rs` | `decode` | The platform's `<epochMillis>:<uuid>` keyset cursor over `(created_at, id)`: encode, and decode with a `400 invalid_request` for anything outside `^\d{1,15}:[0-9a-f-]{36}$`; `Positioned`, the `(created_at, id)` of a row `activity_page::split` pages; later pages reuse it |
| `lifecycle.rs` | `Request` | `project archive\|restore\|pause\|resume` / `POST /projects/{id}/{archive,restore,pause,resume}` (#328): the platform's one lifecycle command with a four-way `Kind`, human-only at lead tier. Loads the row, checks `expectedVersion` (`409 version_conflict`), then the ported transition (`409 invalid_transition`), writes a version-guarded patch and one `project.<verb>d` event under the receipt; a pause reason must be non-blank (`422`) |
| `limits.rs` | `text` | The platform's charter and paging caps (`project-limits.ts`), counted in UTF-16 units, each breach a `422 invalid_request` naming field, reason and limit |
| `list.rs` | `Request` | `project list` / `GET /projects`: a keyset page newest first, filtered by status, health and `includeArchived` |
| `local_only.rs` | `LocalOnly` | The warning a mutation of a transferred project carries — the change stays here and will not reach the other side — looked up by `activity::record` for every mutation; it never refuses |
| `operation_fields.rs` | `ProjectPatch` | The criterion, milestone and charter shapes a plan operation carries (`project-plan-operations.ts`): patches with no identity field, proposed creates with a client UUID, `Option<Nullable<T>>` so an absent patch field and a `null` stay apart, and `null` refused on a field that is not nullable |
| `operation_rules.rs` | `validate` | A parsed operation list's request-level rules: 1–200 operations and at most 262,144 serialized bytes (the platform's `refuseCap` details `{field, reason: cap_exceeded, limit, actual}`), each field's length or range (`422`, named by its platform path), and normalization — UUIDs lowercase (`400` when malformed), `targetDate` as `toISOString()` |
| `operations.rs` | `Operation` | The twelve typed plan operations, internally tagged on `op`, denying unknown keys; `bounded`, the list deserializer that refuses a 2,001st operation at the schema edge (`400 operations is invalid`) on every adapter; `Nullable` and `non_null`, which keep absent, `null` and a value apart |
| `plan.rs` | `Request` | `project plan show` / `GET /projects/{id}/plan` / `project_show` with `view: "plan"` (`Verb::PlanRead`): the platform's `ProjectPlanResponse` at the current version; `plan_view` drops archived milestones, items and criteria and every edge naming an archived item, the projection later plan tasks reuse |
| `propose.rs` | `Request` | `project proposal submit` / `POST /projects/{id}/proposals` / MCP `project_propose` (`Verb::ProposalCreate`): request rules before the store, then in one immediate transaction under the receipt the project checks (archived or closed `409 invalid_transition`, stale base `409 proposal_stale`), the `pending` row, `draft` → `planning` with `version + 1`, and one `project.proposal_submitted` event |
| `proposal_view.rs` | `ProposalView` | The platform's `ProjectProposalView` from a stored row and its review; a corrupt stored JSON column reads as empty, as the platform's read path degrades it |
| `proposals.rs` | `ListRequest` | `project proposal list\|show` / `GET /projects/{id}/proposals[/{proposalId}]` (`Verb::ProposalRead`): a keyset page newest first, optionally one state; one proposal, `404 proposal_not_found` when unknown or of another project |
| `receipt.rs` | `run` | The idempotent-command runner every mutation goes through (#327): a `(principal, idempotencyKey)`-scoped `project_command_receipts` row written in the mutation's own immediate transaction; an exact replay returns the stored response and writes nothing, any other reuse is `409 idempotency_conflict` |
| `principal.rs` | `Principal` | The principal an envelope carries: `user` or `project_agent` plus an id; the `local-operator` and `local-agent` ids |
| `receipt.rs` | `run` | The idempotent-command runner every mutation but hard deletion and transfer import goes through (#327): a `(principal, idempotencyKey)`-scoped `project_command_receipts` row written in the mutation's own immediate transaction; an exact replay returns the stored response and writes nothing, any other reuse is `409 idempotency_conflict` |
| `show.rs` | `Request` | `project show` / `GET /projects/{id}`: one charter; a malformed id is `400`, an unknown one `404 project_not_found` |
| `slug.rs` | `base_slug` | `projects.slug` from the charter name, and the `-2`, `-3`, … disambiguator |
| `timestamp.rs` | `iso` | Epoch milliseconds rendered as `toISOString()`, and `targetDate` parsing (calendar date = UTC midnight, RFC 3339 converted to UTC) |
| `view.rs` | `ProjectView` | The platform's charter view key for key minus `workspaceId`, the batched load of a page's repositories and criteria, `written`, the view a mutation re-reads inside its own transaction, and `stored_json`, the degrade-to-empty reader of a stored JSON array |
| `work_item_fields.rs` | `WorkItemPatch` | A work item as an operation carries it: its kind and priority enums, its assignee, the patch and the proposed create from one field list |
| `work_item_rules.rs` | `create` | A work item's request-level rules, shared by `work_item.create` and `work_item.update` so both are held to the same caps |

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

## Lifecycle

`lifecycle.rs` ports `project-lifecycle-service.ts` (#328). `terminal` means
`completed` or `canceled`; `archived` means `archived_at` is set.

| Verb | Refused when, in order | Patch |
| --- | --- | --- |
| `archive` | archived | `archived_at` = now |
| `restore` | not archived; terminal | `archived_at` = `NULL` |
| `pause` | archived; status is not `active` | `status` = `paused` |
| `resume` | archived; status is not `paused` | `status` = `active` |

- **Version first.** A stale `expectedVersion` is `409 version_conflict`
  with `currentVersion`, before the transition is judged. The `UPDATE`
  repeats the version in its `WHERE`, so a writer that lost a race changes
  nothing.
- **A terminal project can be archived.** The issue text says a terminal
  project refuses all four verbs, but the platform code has no status check
  on `archive`, and the code wins. Its design's AC-13 archives finished
  projects to hide them from the index. So a terminal project refuses
  `restore`, `pause` and `resume`, and `archive` still succeeds.
- **Reasons.** `reason` is optional (≤ 4000 UTF-16 units, `422` past it)
  except on `pause`. There a missing reason is `400` (schema edge) and an
  empty or whitespace-only one `422` (invariant edge). The event payload
  `{reason}` keeps the caller's text untrimmed.
- **Unknown project** is `404 project_not_found`, as for the readers. The
  platform's `invalid_transition` for a vanished row is unreachable there.
- **Digest** is the platform's `{expectedVersion, reason}` plus the project
  id, because receipts are scoped to principal and key only. One key reused
  on another project is a conflict, never a replay of the first project.

## Evidence

`evidence_add` and `evidence_page` port the platform's
`createProjectEvidence` (outside an execution) and `listProjectEvidence`
(comemory.io `b86dec5`), minus verification, which is #348's.

| Kind | Claim | Stored trust |
| --- | --- | --- |
| `commit` | `repo` + `commitSha`, both required | `pending` |
| `pull_request` | `repo` + a numeric `externalId`, both required | `pending` |
| `test_run` | with a `repo` (then `commitSha` and `externalId` required) | `pending` |
| `test_run` | no `repo` | `self_reported` |
| `session`, `decision`, `memory` | `externalId` required | `pending` |
| `deployment`, `external_url` | — | `self_reported` |

- **Initial trust** is the platform's matrix without its verifier: a claim a
  verifier could check is `pending` (`metadata.reason:
  verification_pending`) until #348 resolves it; anything else is the
  principal's word, `self_reported`. Nothing writes `verified` or `invalid`
  yet.
- **Unknown stored trust** reads as `invalid`, the least trusted state, and
  the `trust=invalid` filter keeps it, so the filter agrees with the view.
- **The repository gate is ported:** a `repo` outside the project's own
  repositories is `403 repo_not_allowed`, and nothing is stored.
- **Criterion links:** every id must be a criterion of this project, at
  either level, as the platform checks; repeats link once.
- **The digest includes `projectId`.** The platform's principal is scoped to
  one project, so its body omits it; the engine's local agent reaches every
  project, and one key on two projects would otherwise replay the first.
- **`evidence_not_found`** is not reached here: no attach or list path names
  an evidence id. #348's verify retry is its first caller.
- **No index:** the platform's table has none, and one project's evidence is
  small.
## Proposals

A plan changes only through an immutable proposal reviewed against the plan
version it was written against (`propose.rs`, #336). Ported from the
platform's `project-plan-operations.ts` and `project-proposal-service.ts`:

- **Twelve operations**, each create naming its client UUID so approval
  writes the identity the reviewer saw. Stored normalized (UUIDs lowercase,
  `targetDate` ISO) in the platform's key order.
- **Two refusal tiers, one code.** The schema edge (`400 invalid_request`):
  more than 2,000 operations, a wrong type, a bad enum, an unknown key. The
  documented caps (`422 invalid_request`): more than 200 operations, more
  than 256 KiB of serialized operations, and every field length or range.
  `POST …/proposals` admits a 32 MiB body so the 2,000 bound is the engine's
  answer, not a `413`.
- **Checks before the store:** the key, the shape and every cap. **Inside the
  transaction:** the project exists, is neither archived nor
  completed/canceled (`409 invalid_transition`), and its plan version still
  equals `basePlanVersion` (`409 proposal_stale`). The live-plan preflight
  (references, entity caps after applying, the dependency graph) is #337's.
- **Status:** the first proposal moves a `draft` project to `planning` with
  `version + 1`, in the same transaction as the row and its
  `project.proposal_submitted` event (`{basePlanVersion, operationCount,
  riskCount}`); a later one moves nothing.
- **Digest:** the platform's `digestBody` verbatim — `basePlanVersion,
  operations, rationale, assumptions, risks` as received — which the row
  also stores as `request_digest`.
- **Divergences:** a patch carrying an identity (`id`, or a criterion's
  `workItemId`) is refused `400`, where zod strips the key; a shape error
  inside an operation names `operations.<i>`, because serde buffers an
  internally tagged enum; caps answer `422`, as for the charter.

## Idempotency

Every mutation except hard deletion (#320) and transfer import (#342) —
`create`, `lifecycle`, `evidence_add` and `propose` today — runs through
`receipt::run` (`receipt.rs`, #327), ported from the platform's
`project-command-receipt-service.ts`. Import is idempotent by content instead:
the same bundle again answers `unchanged` and writes nothing, and it never
carries receipts, which stay with the engine that ran each command:

- **Key:** `idempotencyKey` is required on the core request, 1–200 UTF-16
  units; a breach is `422 invalid_request` (the recorded cap divergence). The
  CLI mints one when `--idempotency-key` is omitted, so only a named key is
  retry-safe.
- **Scope:** `(principal_type, principal_id, idempotency_key)` — not the
  command, not the project — so two principals may reuse a key.
- **Digest:** hex SHA-256 of canonical JSON `{"commandType", "body"}`; the
  body is the request as received, before normalization (as the platform
  digests its validated input, not its normalized charter), minus
  `idempotencyKey` and bookkeeping (`workspaceId`). So a retry that respells a
  repository or the id's case is another command. Create's body includes `id`,
  because the engine accepts a client id.
- **Replay:** same command type and digest → the stored core response,
  parsed back into the typed response. Nothing runs and nothing is written:
  no state change, no activity event, no `project_changes` frame (a
  mutation's feed row is written by `activity::record` inside `apply`, which
  a replay never reaches), and no `activity_log` row.
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
  - `owner` adds deletion and transfer import.
  - Transfer (#342, engine-only verbs): `project export` is human-only at
    `member`; `project import` is human-only at `owner`, because a bundle can
    carry rows attributed to principals other than the importer.
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

## Transfer (#342)

The offline core that pull (#344) and `project migrate` (#345) leg over the
network. Two explicit directions, never a mesh:

- **What travels.** Every table `store::schema_projects::transfer_class` marks
  `Carried`. `project_command_receipts` stays, because a receipt replays a
  response only the engine that ran the command produced. The binding stays too,
  because each side records its own. A new project table has no class until
  someone decides, and a test fails until then.
- **Same tasks = same digest.** The digest is SHA-256 over the canonical bundle,
  after any actor remap, so a repeat import is a no-op even after a remap. The
  same id with a different digest is `skipped`, and both copies stay. Nothing
  is ever overwritten.
- **Collisions.** A `key_prefix` or `slug` held by another project is refused
  with `422 invalid_request` (`details.value` names it). A child row whose id
  another project holds is refused with `422 conflict` naming the table. The
  whole import rolls back.
- **No activity event on import.** Activity rows are transferred content, and
  an import event would make the next identical import diverge. A successful
  import still appends one `changed` frame to the #324 feed (the project's own
  id as `event_id`, as a deletion does), so a connected console refetches; the
  feed is outside the transferred tables. `unchanged` and `skipped` append
  nothing. Nothing goes to the replica journal or `sync_log`.
- **Remap.** `ActorRemap {from, to}` rewrites every
  `<role>_principal_type`/`_id` pair equal to `from`, never JSON bodies or
  `verified_by`. The binding records the replaced actor.
- **Bound projects warn.** Every mutation core must copy
  `activity::record(..).local_only` into its response's `warnings`, which is
  omitted when empty. The warning never refuses, and a later local change to a
  transferred project stays local. `create` is exempt, because a new project is
  never bound. #328 is the first verb that surfaces it.

## Command tree

Each later task adds its own group with its verbs; nothing is declared empty.

```text
comemory project
  create | show | list                      #326
  activity                                  #331 (read-only)
  export | import                           #342 (CLI-only; hosted routes are #343)
  archive | restore | pause | resume        #328
  plan show                                 #335 (the committed plan)
  plan …                                    plan slice (#264)
  proposal submit | list | show             #336 (review verbs #338, #339)
  item …          (ready, start, …)         #351
  execution …     (heartbeat, block, resume, request-review)  #352
  packet …                                  packets slice (#266)
  evidence add | list                       #346
  approvals …                               approval inbox
  changes                                   #324 (read-only; HTTP for the relay)
```

## MCP budget

The curated catalog held 15 tools before this epic; projects add eight, and
human-only verbs get none:

- readers: `project_list`, `project_show`. The plan is a `view: "plan"` of
  `project_show` (#335), one activity page a `view: "activity"` with
  `limit`, `cursor` and `order` (#331), and one evidence page a
  `view: "evidence"` with `limit`, `cursor`, `kind`, `trust` and
  `workItemId` (#346), none a row of its own;
- writers: `project_propose` (#336: submits a proposal for human review;
  nothing in the plan changes until a human approves it), `project_work`
  (ready, start), `project_execution` (heartbeat, block, resume, request
  review), `project_work_packet`, `project_evidence` (#346), `project_health`.

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

#351 implements this with `project_work`; #352 (`project_execution`) follows
it. `project_evidence` (#346) is a leaf bound to `project evidence add`: its
group has one writer verb, and the list is `project_show`'s evidence view, so
a `--read-only` session can still read it.
