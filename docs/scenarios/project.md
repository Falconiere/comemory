# `comemory project`

Engine-owned project management (epic #261): charter a project, read it back
and page through the charters — offline, in a fresh data directory, with no
account — archive, restore, pause and resume it, read its committed plan,
page its activity log, poll the body-free change feed, and propose plan
changes for review. Nested: `create` / `show` / `list` / `archive` /
`restore` / `pause` / `resume` / `activity` / `changes` / `plan show` /
`proposal submit|list|show`. Each verb is a thin shell over a
`domains::projects` core that the HTTP routes and the MCP tools
(`project_list`, `project_show`, whose `view: "activity"` reads the
activity page, and `project_propose`) call too. Every core runs under a capability
envelope (#315), chosen by the surface, never by a header or an argument:

- the CLI is the **local operator**, a `user` (`local-operator`) at `owner`
  tier, so it may run every verb;
- MCP and local-mode HTTP are the **local agent**, a `project_agent`
  (`local-agent`) holding all six capabilities and no human verb, so they read
  but cannot create, approve, complete, cancel or delete.

The actor is stored on every row and activity event and reads back as
`createdBy` / `leadUserId`. A charter has no update verb: after creation it
changes only through an approved `project.update` proposal (#338).

Every mutation except hard deletion (#320) is idempotent (#327): `create`
and the four lifecycle verbs today. Each carries an idempotency key, scoped
to the principal, and its first answer is stored as a command receipt in the
same transaction. A retry with the same key and body returns that answer and
writes nothing — no row, no activity event, no change-feed frame, no
`activity_log` row. The body is compared exactly as sent, before
normalization; a lifecycle verb also digests its project id, lowercased, so
the id's case never matters but another project does. The same key with
another body, command or project is refused with `idempotency_conflict`. A failed command
stores no receipt, so a retry runs again. Receipts have no TTL: they live
until their project is hard-deleted.

**Runnable tests:** `tests/cli__project.rs`, `tests/cli__project_receipts.rs`,
`tests/serve__routes__projects.rs`,
`tests/cli_scenario_mcp.rs`, colocated `src/domains/projects/tests/*`,
`src/store/tests/projects.rs`, `src/store/tests/project_read.rs`,
`tests/cli__project_changes.rs`, `src/store/tests/project_changes.rs`,
`tests/cli__project_plan.rs`, `src/store/tests/project_plan.rs`,
`tests/cli__project_activity.rs`, `src/store/tests/project_activity.rs`,
`tests/cli__project_lifecycle.rs`, `tests/serve__routes__project_lifecycle.rs`,
`tests/cli__project_proposal.rs`, `tests/serve__routes__project_proposals.rs`,
`src/store/tests/project_proposals.rs`

**HTTP:** `POST /api/v1/projects` (`403 project_agent_scope` for the local
agent), `GET /api/v1/projects`, `GET /api/v1/projects/{id}`,
`GET /api/v1/projects/{id}/plan`, `GET /api/v1/projects/{id}/activity`,
`POST /api/v1/projects/{id}/{archive,restore,pause,resume}` (`403
project_agent_scope` for the local agent),
`POST|GET /api/v1/projects/{id}/proposals`,
`GET /api/v1/projects/{id}/proposals/{proposalId}`,
`GET /api/v1/projects/changes` — see the
[HTTP API guide](../guides/http-api.md#route-map).

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

`show <ID>`, `archive <ID>`, `restore <ID>`, `pause <ID>`, `resume <ID>`,
`activity <ID>`, `plan show <ID>`, `proposal submit <PROJECT_ID>` and
`proposal list <PROJECT_ID>` — the project's UUID, in either case.
`proposal show <PROJECT_ID> <PROPOSAL_ID>` — the project's and the
proposal's UUIDs.

## Flags

| Flag | Subcommand | Default | Effect |
| --- | --- | --- | --- |
| `--name` | `create` | required | Display name, 1–120 characters; the slug derives from it |
| `--key-prefix` | `create` | required | 2–10 of `A-Z0-9`, starting with a letter; unique in the data directory |
| `--outcome` | `create` | required | The finite outcome, 1–2000 characters |
| `--success-criterion` | `create` | none | A project-level success criterion; repeatable, up to 50 of 1–500 characters |
| `--constraint` | `create` | none | A constraint; repeatable, up to 50 of 1–500 characters |
| `--non-goal` | `create` | none | A non-goal; repeatable, up to 50 of 1–500 characters |
| `--repository` | `create` | none | A canonical `owner/name`; repeatable, up to 50; lowercased and de-duplicated |
| `--lead` | `create` | the local operator | The lead's principal id |
| `--target-date` | `create` | none | `YYYY-MM-DD` (UTC midnight) or an RFC 3339 timestamp |
| `--id` | `create` | minted | Use this UUID as the project id (stored lowercase) |
| `--idempotency-key` | `create` | minted | Retry key, 1–200 UTF-16 units: rerunning with the same key and exactly the same flags prints the first answer and writes nothing; the same key with other flags exits 75 (`idempotency_conflict`). Omitted, a fresh UUID is minted, so that run is not retry-safe |
| `--expected-version` | `archive`, `restore`, `pause`, `resume` | required | The `version` last read (`project show`); a stale one exits 75 (`version_conflict`) |
| `--reason` | `archive`, `restore`, `pause`, `resume` | none | Why, up to 4000 UTF-16 units, stored untrimmed in the event payload. Required for `pause`: missing exits 64 (`reason is required`), empty or whitespace-only exits 65 (`reason is blank`) |
| `--idempotency-key` | `archive`, `restore`, `pause`, `resume` | minted | As for `create`: the same key and flags print the first answer and write nothing; the same key for another verb, version, reason or project exits 75 |
| `--status` | `list` | all | `draft`, `planning`, `active`, `paused`, `completed` or `canceled` |
| `--health` | `list` | all | `unknown`, `on_track`, `at_risk` or `off_track` |
| `--include-archived` | `list` | false | Include archived projects |
| `--cursor` | `list` | first page | The previous page's `nextCursor` (`<epochMillis>:<uuid>`) |
| `--limit` | `list` | `20` | Page size, 1–100 |
| `--order` | `activity` | `desc` | `desc` (newest first) or `asc` (oldest first) |
| `--cursor` | `activity` | first page | The previous page's `nextCursor` (`<epochMillis>:<uuid>`) |
| `--limit` | `activity` | `50` | Page size, 1–200 |
| `--after` | `changes` | `0` | Frames after this `seq` |
| `--limit` | `changes` | `100` | Page size, 1–1000 |
| `--base-plan-version` | `proposal submit` | required | The plan version the operations were written against; another current version exits 75 (`proposal_stale`) |
| `--operations` | `proposal submit` | one of the two | The operations as a JSON array of 1–200 typed plan operations, at most 256 KiB serialized |
| `--operations-file` | `proposal submit` | one of the two | Read that JSON array from a file, or stdin for `-` |
| `--rationale` | `proposal submit` | required | Why the change is proposed, 1–4000 characters |
| `--assumption` | `proposal submit` | none | An assumption; repeatable, up to 50 of 1–500 characters |
| `--risk` | `proposal submit` | none | A risk; repeatable, up to 50 of 1–4000 characters |
| `--idempotency-key` | `proposal submit` | minted | Retry key, as for `create` |
| `--state` | `proposal list` | all | `pending`, `approved`, `changes_requested`, `rejected` or `superseded` |
| `--cursor` | `proposal list` | first page | The previous page's `nextCursor` |
| `--limit` | `proposal list` | `20` | Page size, 1–100 |

## Refusals

Every validation refusal is the platform's `invalid_request` or `project_not_found`:
a limit or rule breach answers `422` (exit 65) naming field, reason and limit
(`name is too_long (limit 120)`); a malformed cursor, filter value or id
answers `400` (exit 64). A refused create writes nothing — no project, no
repository, no criterion, no activity event.

Authority refusals come before validation and before the core touches the
store, and they write nothing, not even the `activity_log` row. The CLI
never even opens its database for one. Over HTTP a malformed body still
answers `400` first, and HTTP and MCP open their connection before the core
runs. The refusals are:

- an agent on a human-only verb: `403 project_agent_scope` (`This command
  requires a signed-in human`);
- an agent missing a capability: `403 project_agent_scope` (`This grant does
  not carry the <capability> capability`);
- a human below the verb's tier: `403 forbidden`. A `member` may not run a
  lead verb (lifecycle, proposal review, health, project completion or
  cancellation), and a `lead` may not delete.

The CLI's operator is never refused, because it holds the `owner` tier. An
unrecognised capability empties an agent's whole capability list, and the
warning never logs the value.

## Scenarios

### project-01 Charter, read back and list offline

- **Flags:** `--name`, `--key-prefix`, `--outcome`, `--success-criterion`,
  `--constraint`, `--non-goal`, `--repository`, `--target-date`, `--json`
- **Setup:** a fresh `COMEMORY_DATA_DIR`, no credential, no network
- **Command:** `comemory project create --name 'Ship offline projects' --key-prefix SHIP --outcome '…' --json`
- **Expect:** a `draft` project with slug `ship-offline-projects`, lead
  `local-operator`, the repository lowercased; `project show <id>` and
  `project list` return the same view; exactly one `project.created` activity
  event is written with the charter.
- **Covered by:** `tests/cli__project.rs::a_project_is_created_shown_and_listed_offline`,
  `tests/cli__project.rs::the_tty_views_render_every_charter_line_and_the_next_page`,
  `src/domains/projects/tests/create.rs::creates_a_draft_charter_with_one_event_and_one_telemetry_row`

### project-02 Every flag reaches the core

- **Flags:** `--id`, `--lead`, `--target-date`, `--status`, `--health`, `--include-archived`
- **Command:** `comemory project create --id <UUID> --lead lead-7 …`, then
  `comemory project list --status draft --health unknown --include-archived`
- **Expect:** the id is stored lowercase, the lead is `lead-7`, an RFC 3339
  target date is converted to UTC; the filters narrow the page; an unknown
  health value exits 64.
- **Covered by:** `tests/cli__project.rs::every_flag_reaches_the_core`,
  `src/domains/projects/tests/list.rs::archived_projects_stay_out_unless_asked_and_filters_narrow`

### project-03 Refusals by edge

- **Flags:** `--key-prefix`, `--name`, `--limit`, `--cursor`
- **Command:** a duplicate `--key-prefix`, a 121-character `--name`,
  `--limit 101`, `--cursor abc`, `project show not-a-uuid`
- **Expect:** exit 65 for the duplicate (`keyPrefix is already used in this
  workspace`), the long name (`name is too_long (limit 120)`) and the page
  size (`limit is too_large (limit 100)`); exit 64 for the cursor (`cursor is
  invalid`), the malformed id and an unknown id (`Project not found`); the
  project count is unchanged. Each cap accepts its limit and refuses one past
  it; a failure after the project row is written rolls every row back.
- **Covered by:** `tests/cli__project.rs::refusals_exit_by_edge_and_leave_no_row`,
  `src/domains/projects/tests/create.rs::every_cap_accepts_its_limit_and_refuses_one_past_it_leaving_no_row`,
  `src/domains/projects/tests/create.rs::a_failure_after_the_project_row_rolls_everything_back`

### project-04 A keyset walk under concurrent inserts

- **Flags:** `--limit`, `--cursor`
- **Setup:** 30 projects, and another process creating more during the walk
- **Command:** `comemory project list --limit 3`, then `--cursor <nextCursor>` until it is `null`
- **Expect:** every project that existed before the walk appears exactly
  once; rows sharing one millisecond page by id.
- **Covered by:** `tests/cli__project.rs::a_walk_returns_every_earlier_project_once_while_another_process_inserts`,
  `src/store/tests/project_read.rs::a_walk_is_total_over_created_at_and_id`

### project-05 The same cores over HTTP and MCP

- **Flags:** none (HTTP and MCP)
- **Setup:** a project chartered through the CLI in the server's or
  session's data directory.
- **Command:**
  - `POST /api/v1/projects`, `GET /api/v1/projects/{id}` and
    `GET /api/v1/projects`;
  - MCP `project_list` / `project_show` in a `comemory mcp --read-only`
    session.
- **Expect:**
  - The local agent's create answers `403 project_agent_scope` and writes no
    row, even for an over-limit charter.
  - The CLI's project reads back unchanged by id and in the list, with
    `createdBy` `local-operator`.
  - A list limit answers `422` with `{field, reason, limit}` details. A
    malformed body, query or cursor answers `400`.
  - A read-only server refuses the create with `405 read_only`.
  - Both MCP readers answer read-only, and their refusals carry `code` and
    `details`.
- **Covered by:** `tests/serve__routes__projects.rs::the_local_agent_is_refused_the_create_and_writes_nothing`,
  `tests/serve__routes__projects.rs::the_agent_reads_back_what_the_operator_chartered`,
  `tests/serve__routes__projects.rs::malformed_input_answers_400_and_a_list_limit_422`,
  `tests/serve__routes__projects.rs::a_read_only_server_refuses_the_create_but_still_lists`,
  `tests/cli_scenario_mcp.rs::mcp_09_project_readers_work_read_only`

### project-06 Who may run which verb

- **Flags:** none (the envelope is the surface's)
- **Setup:** a fresh `COMEMORY_DATA_DIR` with no config file and no login
- **Command:** `comemory project create …`, then `project show` / `project list`
- **Expect:**
  - The operator's id is the actor on the stored row, on the
    `project.created` event and in both reads.
  - An agent holding all six capabilities is refused every human-only verb,
    with no row and no database file opened. A `member` is refused every
    lead verb, and a `lead` is refused deletion.
  - An agent's capability list with one unrecognised value grants nothing.
- **Covered by:** `tests/cli__project.rs::the_unconfigured_operator_is_the_actor_on_the_row_the_event_and_the_reads`,
  `src/domains/projects/tests/authority.rs::every_human_only_verb_refuses_an_agent_holding_all_six_before_the_store`,
  `src/domains/projects/tests/authority.rs::a_member_is_refused_every_lead_verb_and_a_lead_is_refused_delete`,
  `src/domains/projects/tests/authority.rs::an_unrecognised_capability_empties_the_whole_list_and_logs_no_value`,
  `src/mcp/tests/state.rs::the_project_envelope_holds_every_capability_and_no_human_verb`

### project-07 The body-free change feed

- **Flags:** `--after`, `--limit`, `--json`
- **Setup:** projects created through the CLI while a `comemory serve` runs
  on the same data directory
- **Command:** `comemory project changes --after <seq> --json`, or
  `GET /api/v1/projects/changes?after=<seq>&limit=<n>`
- **Expect:** one frame `{seq, entity: "project", project_id, event_id, op}`
  per committed mutation — `op: changed`, `event_id` the mutation's activity
  event — and none for a refused, replayed or rolled-back command; no
  charter text in the bytes. A hard deletion (#320) adds `op: deleted` with
  the project id as `event_id`, and the row outlives the project. A reader
  that drops its connection mid-response or outlives a server restart resumes
  from its last `seq` with no gap and no duplicate. Retention is unbounded; a
  cursor past the head (`after is cursor_ahead`) or below the oldest retained
  row (`after is cursor_expired`) exits 65 (`422`), and `comemory rebuild`
  keeps every `seq`. The TTY view prints one `seq op project_id event_id`
  line per frame and `next: --after <seq>`.
- **Covered by:** `tests/cli__project_changes.rs::each_committed_mutation_emits_one_body_free_frame`,
  `tests/cli__project_changes.rs::a_reader_resumes_after_a_mid_stream_drop_and_a_server_restart`,
  `tests/cli__project_changes.rs::cursors_outside_the_feed_are_refused_over_http_and_the_cli`,
  `tests/cli__project_changes.rs::a_rebuild_keeps_every_seq_so_a_cursor_stays_valid`,
  `src/domains/projects/tests/changes.rs::a_mutation_rolled_back_after_its_feed_row_emits_nothing`,
  `src/store/tests/project_changes.rs::a_deletion_row_and_the_earlier_rows_outlive_the_project_cascade`

### project-08 A retry-safe create

- **Flags:** `--idempotency-key`, `--json`
- **Setup:** a fresh `COMEMORY_DATA_DIR`
- **Command:** `comemory --json project create --name SHIP --key-prefix SHIP --outcome 'Ship it' --idempotency-key k1`, run twice, then once more with another `--outcome`
- **Expect:**
  - The second run prints byte-identical stdout and leaves every project
    table, `project_changes` and `activity_log` unchanged: no change
    frame for a replay.
  - The third run exits 75: `This idempotency key was already used for a
    different command`.
  - Two processes racing one key create one project and one receipt, and
    both print the same answer.
  - A create refused inside its transaction (a taken `--key-prefix`) stores
    no receipt, so the same command succeeds once the other project is
    gone.
  - An empty or 201-character key exits 65 before the database is opened.
  - A receipt the CLI stored replays through the HTTP create path as a
    human with the same body. A body without `idempotencyKey` answers `400`.
- **Covered by:** `tests/cli__project_receipts.rs::a_replay_prints_the_first_answer_and_writes_nothing`,
  `tests/cli__project_receipts.rs::the_same_key_with_another_body_is_an_idempotency_conflict`,
  `tests/cli__project_receipts.rs::two_processes_racing_one_key_create_one_project`,
  `tests/cli__project_receipts.rs::a_create_refused_in_its_transaction_leaves_no_receipt_and_its_retry_runs`,
  `tests/cli__project_receipts.rs::a_key_outside_one_to_two_hundred_characters_is_refused`,
  `src/domains/projects/tests/receipt.rs`,
  `src/serve/routes/tests/projects.rs::a_cli_receipt_replays_through_the_http_create_path`,
  `tests/serve__routes__projects.rs::malformed_input_answers_400_and_a_list_limit_422`

### project-09 Read the committed plan

- **Flags:** `--json`
- **Setup:** a project chartered with `--id`; for the populated case, the
  rows of `tests/fixtures/projects/plan_seed.sql` written straight into its
  `comemory.db` (no command writes a plan before approval, #338)
- **Command:** `comemory project plan show <ID> --json`, or
  `GET /api/v1/projects/{id}/plan`, or MCP `project_show` with
  `"view": "plan"`
- **Expect:**
  - `{plan: {projectId, planVersion, milestones, workItems, criteria,
    dependencies}}`, the platform's `ProjectPlanResponse`. A project before
    its first approval reads `planVersion` 0 with no milestones, work items
    or dependencies; its `criteria` are the charter's success criteria.
  - Milestones by `(position, id)` with an ISO `targetDate`, work items by
    `(position, number)` with their parent and milestone ids, criteria of
    both levels by `(position, id)` (`workItemId` null for a project-level
    one), and every `blocks` edge.
  - An archived milestone, work item or criterion is absent, and so is every
    edge that names an archived item. Archiving a milestone or item archives
    nothing under it, so, as on the platform, a live item can name an
    archived `milestoneId` and a live criterion an archived `workItemId`;
    both are kept.
  - The TTY view prints `plan vN of <id>`, then one line per milestone, item
    (`#n [status] title`), criterion (`[resolution] text (scope)`) and edge.
  - A malformed id exits 64 (`400 invalid_request`, `projectId is invalid`),
    an unknown one exits 64 (`404 project_not_found`). An agent without
    `project.read` is refused with `403 project_agent_scope` before the store
    opens.
- **Covered by:** `tests/cli__project_plan.rs::a_fresh_project_reads_plan_version_zero_with_empty_collections`,
  `tests/cli__project_plan.rs::a_charter_s_success_criteria_are_the_plan_s_criteria_at_version_zero`,
  `tests/cli__project_plan.rs::a_seeded_plan_renders_both_criteria_levels_and_every_live_dependency`,
  `tests/cli__project_plan.rs::a_malformed_or_unknown_id_exits_64_naming_the_refusal`,
  `tests/serve__routes__projects.rs::the_agent_reads_the_plan_the_store_holds`,
  `tests/cli_scenario_mcp.rs::mcp_09_project_readers_work_read_only`,
  `src/domains/projects/tests/plan.rs::an_agent_without_project_read_is_refused_before_the_store_opens`,
  `src/store/tests/project_plan.rs::every_plan_row_comes_back_in_display_order_archived_included`

### project-10 Page a project's activity log

- **Flags:** `--order`, `--cursor`, `--limit`, `--json`
- **Setup:** a project chartered through the CLI, with more events appended
  straight into its `comemory.db` in runs sharing one millisecond, and
  another process that keeps appending events to the same project and
  chartering other projects during the walk
- **Command:** `comemory project activity <ID> --order desc --limit 4`, then
  `--cursor <nextCursor>` until it is `null`, and the same walk with
  `--order asc`. Also `GET /api/v1/projects/{id}/activity`, or MCP
  `project_show` with `"view": "activity"`.
- **Expect:**
  - `{events, nextCursor}`, each event the platform's view:
    `{id, projectId, actorPrincipalType, actorPrincipalId, eventType,
    entityType, entityId, payload, createdAt}`. A fresh project's page is
    its one `project.created` event by `local-operator`, and it is identical
    over the CLI, HTTP `data` and MCP.
  - Every event that existed before the walk began appears exactly once in
    either order, and no other project's event appears. Ties on `createdAt`
    are ordered by `id`, so a page boundary inside a tie drops nothing, and
    the `asc` walk is the `desc` walk reversed at every page size.
  - An exact remainder ends with `nextCursor: null`.
  - A stored payload that is not a JSON object reads as `{}`.
  - The TTY view prints `createdAt  eventType entityType entityId
    actorType:actorId` per event, or `no activity`, then `next page: --cursor
    <c>`.
  - A malformed id (`projectId is invalid`), cursor or order exits 64
    (`400`), as does an unknown id (`Project not found`, `404`).
  - `--limit 0` and `--limit 201` exit 65 (`422`, `limit is too_small (limit
    1)` / `too_large (limit 200)`). 1 and 200 are accepted.
  - An agent without `project.read` is refused `403 project_agent_scope`
    before the store opens.
  - Over MCP, `limit` or `cursor` on the `charter` or `plan` view is
    refused `400 invalid_request` naming the field (`limit is page_only`),
    and `order` on any view but `activity` (`order is activity_only`), so a
    caller never believes it paged a charter.
- **Covered by:** `tests/cli__project_activity.rs::a_walk_returns_every_earlier_event_once_while_another_process_writes`,
  `tests/cli__project_activity.rs::json_and_tty_views_page_one_project_and_refusals_exit_by_edge`,
  `src/store/tests/project_activity.rs::both_orders_walk_the_same_set_in_reverse_at_every_page_size`,
  `src/domains/projects/tests/activity_page.rs`,
  `tests/serve__routes__projects.rs::the_activity_page_matches_the_cli_and_refuses_by_edge`,
  `tests/cli_scenario_mcp.rs::mcp_10_project_activity_view_reads_read_only`

### project-11 Archive, restore, pause and resume

- **Flags:** `--expected-version`, `--reason`, `--idempotency-key`, `--json`
- **Setup:** a fresh `COMEMORY_DATA_DIR`; a project chartered with `create`.
  No verb reaches `active` yet (plan approval, #338, will), so the tests seed
  the status straight into `comemory.db`.
- **Command:** `comemory project pause <ID> --expected-version 1 --reason
  'Design review'`, then `resume`, `archive` and `restore`, each at the
  version the previous answer printed
- **Expect:**
  - Each success prints the project at `version + 1` and writes one
    `project.paused` / `resumed` / `archived` / `restored` event (payload
    `{reason}`) and one change-feed frame in the same transaction. The TTY
    view prints `version` and `archived` lines, so the next
    `--expected-version` needs no `--json`.
  - The legal source states are the platform's
    (`project-lifecycle-service.ts`): `archive` any unarchived project,
    `restore` an archived one that is not `completed` or `canceled`, `pause`
    an unarchived `active` one, `resume` an unarchived `paused` one. Every
    other pair exits 75 (`invalid_transition`) with the platform's sentence
    and changes nothing. A terminal project can still be archived, because
    the platform code allows it, but never restored, paused or resumed.
  - A stale `--expected-version` exits 75 (`The project has changed since it
    was last read`); a missing pause reason exits 64 and a blank one 65; an
    unknown id exits 64 (`Project not found`).
  - The archived project leaves `project list` and returns with
    `--include-archived` or after `restore`.
  - A named key replays; the same key with another verb exits 75
    (`idempotency_conflict`).
  - Over HTTP the local agent is refused every verb with `403
    project_agent_scope` and nothing written; a read-only server answers
    `405`. An agent envelope is refused before the database is opened, and a
    `member` with `403 forbidden`.
- **Covered by:** `tests/cli__project_lifecycle.rs::a_project_walks_its_lifecycle_offline`,
  `tests/cli__project_lifecycle.rs::a_named_key_replays_and_an_unknown_project_is_not_found`,
  `src/domains/projects/tests/lifecycle.rs::every_source_state_against_every_verb`,
  `src/domains/projects/tests/lifecycle.rs::a_blank_missing_or_over_long_reason_is_refused_and_changes_nothing`,
  `src/domains/projects/tests/lifecycle.rs::a_stale_version_is_a_conflict_carrying_the_current_version`,
  `src/domains/projects/tests/lifecycle.rs::an_agent_is_refused_before_the_store_is_opened`,
  `src/domains/projects/tests/lifecycle.rs::a_replay_writes_nothing_and_any_other_reuse_of_the_key_conflicts`,
  `src/store/tests/projects.rs::a_lifecycle_patch_lands_only_at_the_version_it_names`,
  `tests/serve__routes__project_lifecycle.rs`

### project-12 Attach and page typed evidence

- **Flags:** `add`: `--kind`, `--source`, `--work-item`, `--external-id`,
  `--url`, `--repo`, `--commit-sha`, `--metadata`, `--criterion`,
  `--idempotency-key`; `list`: `--kind`, `--trust`, `--work-item`,
  `--cursor`, `--limit`; both `--json`
- **Setup:** a project chartered with `--id` and `--repository
  falconiere/comemory`, its committed plan seeded from
  `tests/fixtures/projects/plan_seed.sql` (work items and criteria)
- **Command:** `comemory project evidence add <ID> --kind commit --source git
  --repo falconiere/comemory --commit-sha <sha> --work-item <item>`, then
  `comemory project evidence list <ID> --trust pending --json`; over HTTP
  `POST|GET /api/v1/projects/<ID>/evidence`, over MCP `project_evidence` and
  `project_show` with `view: "evidence"`
- **Expect:**
  - An attach without `--work-item` is project-level (`workItemId: null`);
    with it, the row names that item. Each attach writes one row, its
    criterion links, one `project.evidence.recorded` event and one change
    frame, and prints the platform's evidence view.
  - Nothing is verified yet (#348): a `commit`, `pull_request`, a `test_run`
    naming a repository, and a `session`, `decision` or `memory` with an
    external id are stored `pending`; a `test_run` without a repository, a
    `deployment` and an `external_url` are `self_reported`.
  - `list` pages newest first; `--kind`, `--trust` and `--work-item` filter
    alone or together, a `--limit` walk visits every row once and ends with
    `nextCursor: null`, and the page is identical over the CLI, HTTP `data`
    and MCP.
  - A stored trust this build does not know reads as `invalid`, and
    `--trust invalid` returns it.
  - A malformed `--repo` or `--commit-sha` exits 65 (`422 … invalid_format`,
    or `too_long (limit 256)`), a claim missing what its kind needs exits 65
    (`repo is required_for_commit`, …), an unknown `--kind` exits 64, a
    repository outside the project's exits 70 (`403 repo_not_allowed`), an
    unknown `--work-item` or project exits 64 (`404`) — each storing
    nothing, not even a receipt.
  - Rerunning with the same `--idempotency-key` and flags prints the first
    answer; the key with another body exits 75 (`409
    idempotency_conflict`).
  - Over MCP, a `--read-only` session refuses `project_evidence` with
    `read_only` and still pages through `project_show`; `kind`, `trust` or
    `workItemId` on another view is `400` (`kind is evidence_only`).
- **Covered by:** `tests/cli__project_evidence.rs::evidence_attaches_at_both_levels_and_every_filter_pages_the_right_rows`,
  `tests/cli__project_evidence.rs::a_retried_key_prints_the_first_answer_and_a_reused_one_conflicts`,
  `tests/cli__project_evidence.rs::every_refusal_exits_by_its_edge_and_stores_nothing`,
  `tests/cli__project_evidence.rs::tty_views_print_an_empty_page_and_an_attached_record`,
  `src/store/tests/project_evidence.rs`,
  `src/domains/projects/tests/{evidence,evidence_check,evidence_add,evidence_page}.rs`,
  `tests/serve__routes__projects.rs::evidence_attaches_and_pages_over_http_and_refuses_by_code`,
  `tests/cli_scenario_mcp.rs::mcp_11_project_evidence_attaches_and_pages`
### project-12 Propose plan changes, then list and show proposals

- **Flags:** `--json`
- **Setup:** a draft project chartered with `--id`; the twelve-kind operation
  list in `tests/fixtures/projects/proposal_operations.json`
- **Command:** `comemory project proposal submit <ID> --base-plan-version 0
  --operations-file ops.json --rationale '…'`, then `proposal list <ID>
  --state pending` and `proposal show <ID> <PROPOSAL_ID>`; or `POST|GET
  /api/v1/projects/{id}/proposals[/{proposalId}]`; or MCP `project_propose`
- **Expect:**
  - `{proposal: {id, projectId, basePlanVersion, state, operations,
    rationale, assumptions, risks, proposerPrincipalType,
    proposerPrincipalId, createdAt, updatedAt, review}}`, the platform's
    `ProjectProposalView`; `state` is `pending` and `review` is `null` until
    a review exists. The operations come back as submitted, with every UUID
    lowercase and every `targetDate` rendered as an ISO timestamp; an absent
    patch field stays absent and a `null` stays `null`; `null` on a field
    the platform does not declare nullable is refused `400`.
  - The twelve operations: `project.update`, `criterion.create|update|archive`,
    `milestone.create|update|archive`, `work_item.create|update|archive`,
    `dependency.add|remove`. A create carries its client UUID; a patch cannot
    carry an identity (`id`, or a criterion's `workItemId`) and is refused.
  - The first proposal moves a `draft` project to `planning` and bumps its
    `version` in the same transaction as the row, one
    `project.proposal_submitted` event and one change-feed frame; a later
    proposal or a replay moves nothing.
  - The list is newest first, filtered by `--state`, paged by `nextCursor`
    (1–100 per page).
  - Refusals: 2,001 operations `400` (exit 64, `operations is invalid`) — over
    HTTP too, even past the server's 5 MiB default body limit, since the route
    admits 32 MiB; 201 operations or more than 256 KiB of serialized
    operations `422` (exit 65) with `{field, reason: cap_exceeded, limit,
    actual}`; a shape error inside operation `i` `400 operations.<i> is
    invalid`; a stale base `409 proposal_stale` (exit 75); an archived,
    completed or canceled project `409 invalid_transition` (exit 75); an
    unknown or foreign proposal `404 proposal_not_found`. None writes a row.
  - An agent without `proposal.create` is refused `403 project_agent_scope`
    before the store opens; the local agent holds it.
- **Covered by:** `tests/cli__project_proposal.rs`,
  `tests/serve__routes__project_proposals.rs`,
  `tests/cli_scenario_mcp.rs::mcp_12_project_propose_submits_and_read_only_refuses`,
  `src/domains/projects/tests/propose.rs`, `src/domains/projects/tests/proposals.rs`,
  `src/domains/projects/tests/operations.rs`,
  `src/domains/projects/tests/operation_rules.rs`,
  `src/store/tests/project_proposals.rs`
