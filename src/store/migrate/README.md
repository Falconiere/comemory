# store/migrate/

**What belongs here:** the pieces of the schema migration runner that
outgrew `src/store/migrate.rs` itself — the single `MIGRATIONS` slice every
consumer reads from, and the preflight/snapshot safety net that runs before
it.

**What does NOT belong here:** the migration SQL text (`migrations/`),
the runner (`run`, `apply`, `set_version`) or the `M_BOOTSTRAP..M_V13`
replay consts, all of which stay in `src/store/migrate.rs` beside this
folder — no `mod.rs` barrel.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `backup.rs` | `snapshot` | `pub(crate)` `VACUUM INTO` snapshot helpers: `snapshot` (through a live connection), `snapshot_path` (opens its own plain connection — used by `comemory rebuild`'s pre-swap snapshot, `src/domains/maintenance/rebuild.rs`), `prune` (prefix-scoped `KEEP = 2` budget), and stale-`.bak` `PRAGMA quick_check` validation before trusting an existing snapshot |
| `historical_document_revision.rs` | `repair` | v28 post-pass: rebuild the historical staged `remote_document` layout into the released accepted-revision layout, preserving accepted rows and removing staged or superseded rows and their search entries in one transaction |
| `list.rs` | `MIGRATIONS` | `Class` + `Migration` + the `MIGRATIONS` const — the single source `run()` iterates, the replay test helpers slice, and the migration-integrity tests classify against |
| `preflight.rs` | `preflight` | `pub(crate)` guard run from `connection::open` before `migrate::run`: recognizes the historical `0026_repository_approval` marker (its DDL moved into migration 25), refuses other unknown applied markers with `Error::SchemaTooNew`, and snapshots via `backup` before pending migrations — a snapshot failure is mandatory-refuse (`Error::Migration`) only when the pending set includes a `Destructive` migration, advisory (warn + proceed) when every pending migration is `Additive` |

When you add a file here, add its row above so the index stays current.
