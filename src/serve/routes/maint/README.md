# serve/routes/maint/

**What belongs here:** the mutating half of the `/api/v1` maintenance
surface — the prune/gc routes and the administrative ones — kept beside the
read-only `GET /doctor` / `GET /consolidate` pair rather than inside it,
because every route here is confirm-gated, job-backed, or both.

**What does NOT belong here:** the read-only maintenance reports, which stay
in `src/serve/routes/maint.rs`, and the pruning logic itself, which is
`prune::`'s.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `doctor.rs` | `table_entries` | `GET /doctor/system` (facts, never runs the embed command), `POST /doctor/rebuild` (alias of the rebuild job), job-backed `POST /doctor/reembed` |
| `gc.rs` | `table_entries` | `GET\|PUT /gc/policy` and the confirm-gated, job-backed `POST /gc/run` |
| `admin.rs` | `table_entries` | `POST /mine`, `POST /hooks/install`, and the job-backed `POST /rebuild` with its shared-connection swap |
| `prune.rs` | `table_entries` | `GET\|POST /prune` and `POST /gc`, plus `split_confirm` — the raw-body confirm-field extractor every confirm-gated route with a real `Request` type reuses; also mounts `GET /prune/candidates` (an alias onto the same handler) and owns `split_dry_run`, the HTTP-only `dry_run` inverse of `apply` on `POST /prune` |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/serve/routes/maint.rs`
(`pub mod <name>;`) and callers import concrete paths.
