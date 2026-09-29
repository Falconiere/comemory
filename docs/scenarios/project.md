# `comemory project`

Engine-owned project management (epic #261): charter a project, read it back
and page through the charters — offline, in a fresh data directory, with no
account. Nested: `create` / `show` / `list`. Each verb is a thin shell over a
`domains::projects` core that the HTTP routes and the MCP readers
(`project_list`, `project_show`) call too. The CLI acts as the local operator
(`user` / `local-operator`). A charter has no update verb: after creation it
changes only through an approved `project.update` proposal (#338).

**Runnable tests:** `tests/cli__project.rs`, `tests/serve__routes__projects.rs`,
`tests/cli_scenario_mcp.rs`, colocated `src/domains/projects/tests/*`,
`src/store/tests/projects.rs`, `src/store/tests/project_read.rs`

**HTTP:** `POST /api/v1/projects` (`201`), `GET /api/v1/projects`,
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

Every refusal is the platform's `invalid_request` or `project_not_found`:
a limit or rule breach answers `422` (exit 65) naming field, reason and limit
(`name is too_long (limit 120)`); a malformed cursor, filter value or id
answers `400` (exit 64). A refused create writes nothing — no project, no
repository, no criterion, no activity event.

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
  `src/domains/projects/tests/create.rs::creates_a_draft_charter_with_one_event_and_one_telemetry_row`

### project-02 Every flag reaches the core

- **Flags:** `--id`, `--lead`, `--target-date`, `--status`, `--health`, `--include-archived`
- **Command:** `comemory project create --id <UUID> --lead lead-7 …`, then
  `comemory project list --status draft --health unknown --include-archived`
- **Expect:** the id is stored lowercase, the lead is `lead-7`, an RFC 3339
  target date is converted to UTC; the filters narrow the page; an unknown
  health value exits 64.
- **Covered by:** `tests/cli__project.rs::every_flag_reaches_the_core`,
  `src/domains/projects/tests/list.rs::a_walk_returns_every_project_once_and_refusals_use_their_edge`

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
- **Command:** `POST /api/v1/projects`, `GET /api/v1/projects/{id}`,
  `GET /api/v1/projects`; MCP `project_list` / `project_show` in a
  `comemory mcp --read-only` session
- **Expect:** `201` then the same view by id and in the list; limits `422`
  with `{field, reason, limit}` details, a malformed body, query or cursor
  `400`; a read-only server refuses the create with `405 read_only`; both MCP
  readers answer read-only, their refusals carrying `code` and `details`.
- **Covered by:** `tests/serve__routes__projects.rs::create_show_and_list_agree_and_create_answers_201`,
  `tests/serve__routes__projects.rs::limits_answer_422_and_malformed_input_answers_400`,
  `tests/serve__routes__projects.rs::a_read_only_server_refuses_the_create_but_still_lists`,
  `tests/cli_scenario_mcp.rs::mcp_09_project_readers_work_read_only`
