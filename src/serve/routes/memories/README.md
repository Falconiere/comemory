# serve/routes/memories/

**What belongs here:** the parts of the `/api/v1` memory resource that do not
fit beside the plain listing routes — the free-text retrieval surface and the
mutating writes, each with its own gates.

**What does NOT belong here:** `GET /memories` and `GET /memories/{id}`,
which stay in `src/serve/routes/memories.rs`, and ranking itself, which is
`retrieval::`'s.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `edit.rs` | `table_entries` | `PATCH /memories/{id}`, `POST /memories/{id}/restore`, `POST /memories/{id}/references/refresh` (`api::{update,restore,refresh_refs}`) |
| `search.rs` | `router` | `GET\|POST /memories/search` (`api::search`) and `GET\|POST /context` (`api::context`), including access-tracking suppression |
| `write.rs` | `table_entries` | `POST /memories` (`api::save`), the confirm-gated `DELETE /memories/{id}`, and `POST /feedback` |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from
`src/serve/routes/memories.rs` (`pub mod <name>;`) and callers import
concrete paths.
