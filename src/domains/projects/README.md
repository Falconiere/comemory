# domains/projects/

**What belongs here:** engine-owned project management (epic #261), ported from
the comemory.io platform's `/v1/projects` contract: the charter and its limits,
slug derivation, the keyset cursor, the project activity writer, and the command
cores the CLI, the loopback HTTP server and the MCP catalog all call.

**What does NOT belong here:** SQL (every project statement lives in
`store::{projects,project_read,project_activity}`, the declared tables in
`store::schema_projects`), delivery (no file here imports `cli`, `serve` or
`mcp`), and the refusal vocabulary, which is `utilities::project_error`.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `activity.rs` | `record` | The one `project_activity_events` writer every mutation shares, called inside the mutation's own transaction; never `activity_log`, which is command telemetry |
| `charter.rs` | `validate` | A create request checked against every charter rule before any store access, in the platform's field order, and normalized for storage (lowercase id, canonical de-duplicated repositories, the lead) |
| `create.rs` | `run` | `project create` / `POST /projects`: the draft charter, its repositories, its project-level criteria and one `project.created` event in one immediate transaction; the slug retried with `-2`, `-3`, … on a real collision |
| `keyset.rs` | `decode` | The platform's `<epochMillis>:<uuid>` keyset cursor over `(created_at, id)`: encode, and decode with a `400 invalid_request` for anything outside `^\d{1,15}:[0-9a-f-]{36}$`; later pages reuse it |
| `limits.rs` | `text` | The platform's charter and paging caps (`project-limits.ts`), counted in UTF-16 units, each breach a `422 invalid_request` naming field, reason and limit |
| `list.rs` | `run` | `project list` / `GET /projects`: a keyset page newest first, filtered by status, health and `includeArchived` |
| `principal.rs` | `Principal` | The actor a core runs as: `user` or `project_agent` plus an id; `local_operator()` until the capability envelope (#315) |
| `show.rs` | `run` | `project show` / `GET /projects/{id}`: one charter; a malformed id is `400`, an unknown one `404 project_not_found` |
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
- **Identities are client-generated UUIDs**, minted when absent, stored lowercase.
- **Repositories are shape-checked only** (canonical `owner/name`); the allow-list
  rule is #337's.

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
  feed …                                    change feed (#270)
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
