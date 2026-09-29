# `comemory project`

Engine-owned project management (epic #261): charter a project, read it back
and page through the charters — offline, in a fresh data directory, with no
account, read a project's committed plan, and poll the body-free change
feed. Nested: `create` / `show` / `list` / `changes` / `plan show`. Each verb is a thin shell over a
`domains::projects` core that the HTTP routes and the MCP readers
(`project_list`, `project_show`) call too. Every core runs under a capability
envelope (#315), chosen by the surface, never by a header or an argument:

- the CLI is the **local operator**, a `user` (`local-operator`) at `owner`
  tier, so it may run every verb;
- MCP and local-mode HTTP are the **local agent**, a `project_agent`
  (`local-agent`) holding all six capabilities and no human verb, so they read
  but cannot create, approve, complete, cancel or delete.

The actor is stored on every row and activity event and reads back as
`createdBy` / `leadUserId`. A charter has no update verb: after creation it
changes only through an approved `project.update` proposal (#338).

Every mutation except hard deletion (#320) is idempotent (#327); `create` is
the first. It carries an idempotency key, scoped to the principal, and its
first answer is stored as a command receipt in the same transaction. A retry
with the same key and body returns that answer and writes nothing — no row,
no activity event, no change-feed frame, no `activity_log` row. The body is
compared exactly as sent, before normalization. The same key with another
body or command is refused with `idempotency_conflict`. A failed command
stores no receipt, so a retry runs again. Receipts have no TTL: they live
until their project is hard-deleted.

**Runnable tests:** `tests/cli__project.rs`, `tests/cli__project_receipts.rs`,
`tests/serve__routes__projects.rs`,
`tests/cli_scenario_mcp.rs`, colocated `src/domains/projects/tests/*`,
`src/store/tests/projects.rs`, `src/store/tests/project_read.rs`,
`tests/cli__project_changes.rs`, `src/store/tests/project_changes.rs`,
`tests/cli__project_plan.rs`, `src/store/tests/project_plan.rs`

**HTTP:** `POST /api/v1/projects` (`403 project_agent_scope` for the local
agent), `GET /api/v1/projects`, `GET /api/v1/projects/{id}`,
`GET /api/v1/projects/{id}/plan`, `GET /api/v1/projects/changes` — see the
[HTTP API guide](../guides/http-api.md#route-map).

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

`show <ID>` and `plan show <ID>` — the project's UUID, in either case.

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
| `--status` | `list` | all | `draft`, `planning`, `active`, `paused`, `completed` or `canceled` |
| `--health` | `list` | all | `unknown`, `on_track`, `at_risk` or `off_track` |
| `--include-archived` | `list` | false | Include archived projects |
| `--cursor` | `list` | first page | The previous page's `nextCursor` (`<epochMillis>:<uuid>`) |
| `--limit` | `list` | `20` | Page size, 1–100 |
| `--after` | `changes` | `0` | Frames after this `seq` |
| `--limit` | `changes` | `100` | Page size, 1–1000 |

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
