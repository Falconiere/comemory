# domains/projects/

**What belongs here:** engine-owned project management (epic #261), ported from
the comemory.io platform's `/v1/projects` contract: the charter and its limits,
slug derivation, the keyset cursor, the project activity writer, and the command
cores the CLI, the loopback HTTP server and the MCP catalog all call.

**What does NOT belong here:** SQL (every project statement lives in
`store::{projects,project_activity}`, the declared tables in
`store::schema_projects`), delivery (no file here imports `cli`, `serve` or
`mcp`), and the refusal vocabulary, which is `utilities::project_error`.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `keyset.rs` | `decode` | The platform's `<epochMillis>:<uuid>` keyset cursor over `(created_at, id)`: encode, and decode with a `400 invalid_request` for anything outside `^\d{1,15}:[0-9a-f-]{36}$` |
| `limits.rs` | `text` | The platform's charter and paging caps (`project-limits.ts`), counted in UTF-16 units, each breach a `422 invalid_request` naming field, reason and limit |
| `principal.rs` | `Principal` | The actor a core runs as: `user` or `project_agent` plus an id; `local_operator()` until the capability envelope (#315) |
| `slug.rs` | `base_slug` | `projects.slug` from the charter name, and the `-2`, `-3`, … disambiguator retried on a real unique-index collision |
| `timestamp.rs` | `iso` | Epoch milliseconds rendered as `toISOString()`, and `targetDate` parsing (calendar date = UTC midnight, RFC 3339 converted to UTC) |
