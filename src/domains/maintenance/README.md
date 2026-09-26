# domains/maintenance/

**What belongs here:** operational corpus maintenance — the four concerns whose
subject is the installation itself rather than any memory in it. *Health*
probes the machine, database and vector extension. *Retention* finds what
should leave and applies it only on an explicit confirmation. *Repair* rebuilds
the SQLite mirror from the markdown of record and re-vectorizes what the
embedder changed underneath. *Upgrade* replaces this binary.

One named file per operation. Maintenance is a capability, not a drawer: a new
operation gets its own file and its own row below, never a shared
`operations.rs`.

**What does NOT belong here:** persistence and rendering. The
`ATTACH`/copy/`DETACH` preservation unit, the pre-rebuild `VACUUM INTO`
snapshot, the schema journal and the migration chain all belong to
[`store/`](../../store/README.md); this capability composes them. TTY and
`--json` emission belongs to `cli/output/`; HTTP status mapping, the response
envelope and the read-only/confirm gates belong to `serve/routes/maint/`. Nor
does another capability's policy: the dashboards call `domains::code::repos`,
`domains::memories::list` and `domains::maintenance::stats` rather than
restating what those own.

All persistence stays in the store: `doctor/system.rs` uses the shared schema
version reader, and `stats.rs` selects `store::stats_counts::Corpus` variants
rather than passing SQL predicates to its counters.

`upgrade` is deliberately CLI-only (`serve::routes::meta::CLI_ONLY`, beside
`serve` and `auth`). A server replacing its own binary on request is not a
feature, so no route reaches it.

## Contents

One row per file, named after its primary item. Command cores take a plain
`Request` and return a plain `Response`/`Report` through
[`utilities::context::Ctx`](../../utilities/README.md), so `cli::` and
`serve::routes::` share one implementation.

| File | Primary item | Purpose |
| --- | --- | --- |
| `activity.rs` | `Request`, `Response`, `Item`, `run`, `MAX_LIMIT` | `GET /api/v1/activity` — one filtered, newest-first page of recorded command runs plus its per-command rollups and the stream cursor. Keeps the must-not-create-the-db invariant: no database, no read, none created |
| `backup.rs` + `backup_restore.rs`, `backup_swap.rs`, `backup_merge.rs` | `create`, `restore`, `merge_erasures` | `comemory backup create\|restore\|merge-erasures` (#256) — CLI-only. `create` writes a `VACUUM INTO` copy of the database, `memories/` with `.trash/`, and `backup.json` under `memory-save.lock`. `restore` records `restore.pending`, stages the snapshot (migrating an older one forward), mints a new epoch into `replica/identity.json` (reason `restore`) with this directory's device id, merges the erasure manifest (the full `erase::apply_locked` per entity, no manifest append, then every digest barred) or marks it `erasure_unknown`, snapshots what it replaces to `comemory.db.pre-restore.bak` / `memories.pre-restore/`, then swaps: markdown moved, database copied in place (`store::replace_in_place`); a rerun finishes a pending swap. `merge_erasures` installs an established manifest, merges it into the live store and clears the restore state. Flat siblings, no subfolder |
| `consolidate.rs` | `run` | `comemory consolidate` / `GET /api/v1/consolidate` — the advisory near-duplicate cluster core: scan, cluster, page. Read-only end to end |
| `consolidation.rs` + [`consolidation/`](consolidation/README.md) | `detect` | The near-duplicate algorithm: union-find grouping of live SimHashes within a Hamming radius, keeper ordering, in-cluster supersede resolution |
| `consolidation_report.rs` | `Report` | The owned value `consolidate::run` returns for both delivery surfaces |
| `doctor.rs` + [`doctor/`](doctor/README.md) | `run` | `comemory doctor` / `GET /api/v1/doctor` — the structured health report, plus `GET /api/v1/doctor/system`. Probes without ever creating the database, and falls back to a read-only open when the schema is newer than this binary. Also the read/probe `domains::integrations::setup::detect` consults once a `comemory.db` exists |
| `erase.rs` + `erase_memory.rs`, `erase_document.rs` | `run`, `apply_locked`, `settle` | `comemory erase` / `POST /api/v1/erase` — permanently erase one memory or document from every table, journal copy and the markdown tree, keeping feed rows, revisions, receipts and digests as the barrier that answers a later offer `payload_erased`. `run` takes `memory-save.lock`, then `replica/identity.lock`; `apply_locked` is the lock-free transaction (tombstone when live and shared, rows, payload bytes, scratch and staged copies, pending uploads withdrawn), which with a `Ledger` appends the entity to the erasure manifest (`replica::identity::record`) before it commits; `settle` is the post-commit FTS `optimize`, derived refresh and truncating checkpoint. The two flat siblings hold the memory and document halves (no subfolder here) |
| `gc.rs` | `run` | `comemory gc` / `POST /api/v1/gc` — reap aged trash entries and purge their mirror rows, then age out learning telemetry. Never creates the database |
| `gc_policy.rs` | `get` | `GET\|PUT /api/v1/gc/policy` — the retention windows and the last gc run |
| `overview.rs` | `run` | `GET /api/v1/overview`, `GET /api/v1/overview/eval-series` — the console landing aggregate, composed from the cores that own each fact |
| `prune.rs` | `run` | `comemory prune` / `GET\|POST /api/v1/prune` — the dry-run candidate report (GET forces `apply=false`), and the confirmed apply that soft-deletes only the listed ids |
| `rebuild.rs` + [`rebuild/`](rebuild/README.md) | `run` | `comemory rebuild` / `POST /api/v1/rebuild` — build a fresh mirror beside the live file, preserve what markdown cannot reconstruct, snapshot the pre-rebuild database, then replace the live content in place through SQLite's backup API while `memory-save.lock` pauses markdown writers |
| `reembed.rs` | `run` | `POST /api/v1/doctor/reembed` — re-vectorize memories and code through the configured embed command, with the shared progress/log/cancel sink and a dimension guard |
| `retention.rs` + [`retention/`](retention/README.md) | `detect` | Stale-memory and ghost-reference detection: orphaned trash, low-value memories, ghost `references_symbol` anchors. Read-only and side-effect free by construction |
| `retention_report.rs` | `Report` | The owned value `prune::run` returns for both delivery surfaces |
| `stats.rs` | `run` | `comemory stats` / `GET /api/v1/stats` — corpus counters and database size. Never creates the database |
| `upgrade.rs` + [`upgrade/`](upgrade/README.md) | `run` | `comemory upgrade` — detect the installation channel, resolve the newest release, install and verify. CLI-only |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/domains/maintenance.rs`
(`pub mod <name>;`) and callers import concrete paths.

## Tests

Mirror tests for the command cores live in `tests/` beside them; each
algorithm folder keeps its own `tests/`. Real old and new SQLite files, real
markdown corpora, real git repositories and the real backup/replace sequence —
no mock stands in for a database. The full `comemory rebuild` journey over a
real indexed document corpus is the crate-root `tests/cli__rebuild_3.rs`, and
the upgrade end-to-end path (a loopback stand-in for GitHub Releases, the real
`install.sh`, a private copy of the binary replaced in place) is
`tests/cli__upgrade.rs`.
