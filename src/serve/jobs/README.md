# serve/jobs/

**What belongs here:** the background job model for the long-running
commands (`index-code`, `ingest-code`, `index`, `rebuild`, `eval`, `tune`,
`bandit`) — the in-process job table, its status watch channels, and the
spawner that runs one `api::<cmd>::run` off the request path. Lifecycle is
`Queued → Running → Done | Error`, not persisted: a server restart forgets
every unfinished job, and there is no cancellation in v1.

**What does NOT belong here:** the routes that accept a job (`202` +
`Location`) or stream its events — those live in `serve/routes/jobs.rs` and
each resource's own route file — and the command logic itself, which is
`api::`'s.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `registry.rs` | `MAX_FINISHED` | The job table: one record per accepted job, a retained `watch::Sender<JobStatus>` (+ a second progress-only channel) per job so a late SSE subscriber still replays a terminal status, and bounded finished-job retention |
| `worker.rs` | `spawn_job` | The one place a background job is started: register `Queued`, await the write permit (mutating jobs only), mark `Running`, run the closure on `spawn_blocking`, record the terminal status. `spawn_job_with_id` is the same for a caller whose closure needs its own job id first; `RegistryProgressSink` is the `api::index_code::ProgressSink` that writes into the registry |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/serve/jobs.rs` (`pub mod
<name>;`) and callers import concrete paths.
