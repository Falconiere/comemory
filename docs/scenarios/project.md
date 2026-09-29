# `comemory project`

Engine-owned project management (epic #261): charter a project, read it back
and page through the charters — offline, in a fresh data directory, with no
account. Nested: `create` / `show` / `list`. Each verb is a thin shell over a
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

**Runnable tests:** `tests/cli__project.rs`, `tests/serve__routes__projects.rs`,
`tests/cli_scenario_mcp.rs`, colocated `src/domains/projects/tests/*`,
`src/store/tests/projects.rs`, `src/store/tests/project_read.rs`

**HTTP:** `POST /api/v1/projects` (`403 project_agent_scope` for the local
agent), `GET /api/v1/projects`,
`GET /api/v1/projects/{id}` — see the
[HTTP API guide](../guides/http-api.md#route-map).

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

`show <ID>` — the project's UUID, in either case.

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
| `--status` | `list` | all | `draft`, `planning`, `active`, `paused`, `completed` or `canceled` |
| `--health` | `list` | all | `unknown`, `on_track`, `at_risk` or `off_track` |
| `--include-archived` | `list` | false | Include archived projects |
| `--cursor` | `list` | first page | The previous page's `nextCursor` (`<epochMillis>:<uuid>`) |
| `--limit` | `list` | `20` | Page size, 1–100 |

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
