# Upgrading comemory

**Goal:** understand what happens to `~/.comemory/comemory.db` when you install
a newer `comemory`, where the safety net lives, and how to use it if something
goes wrong.

## Upgrades are automatic

There is no `comemory migrate` command. Install a newer binary, run any
command, and the schema upgrades in place on that first call — `search`,
`save`, `doctor`, whichever you happen to run next:

```bash
brew upgrade comemory   # or: re-run the shell installer / cargo install --path .
comemory doctor         # first command after the upgrade migrates the DB
```

Keeping migration implicit in `store::connection::open` (rather than a
separate command) means there is no second path a script or muscle memory can
skip — every command migrates the same way, every time.

## What a schema upgrade does

Before the migration chain touches your database, a preflight guard runs:

1. **It checks whether the database is safe to open at all.** comemory reads
   the schema markers already recorded in `schema_meta` and compares them
   against every migration this build knows about. If your database carries
   a marker this build has never heard of, it was written by a *newer*
   comemory than the one you're running — that command refuses with a clear
   error and exits `70`, and nothing is written. Point `COMEMORY_DATA_DIR` at
   a different directory, or install a comemory at least as new as the one
   that last touched this database.
2. **If migrations are pending and any of them could destroy data**
   (dropping a table, or rewriting rows in place), comemory snapshots the
   whole database first with SQLite's `VACUUM INTO` — safer than a raw file
   copy, since it captures committed writes still sitting in the WAL file
   rather than only what's been checkpointed to the main file. You see one
   line on stderr before it starts:

   ```
   comemory: snapshotting database to /home/you/.comemory/comemory.db.pre-v12.bak before migrating (set COMEMORY_SKIP_MIGRATION_BACKUP=1 to skip)
   ```

   Only then does the migration chain run.

An upgrade with only additive migrations pending (adding a column or table,
nothing dropped or rewritten) skips the snapshot notice entirely and migrates
silently — the safety net only announces itself when it's actually needed.
An already-current database costs one marker read and one version read, both
against `schema_meta`, and nothing else: no snapshot, no write.

## Where snapshots go, and how to restore one

Snapshots land next to your database, named after the schema version they
preserve:

```
~/.comemory/comemory.db.pre-v12.bak
```

`comemory rebuild` uses the same mechanism under a fixed name,
`comemory.db.pre-rebuild.bak`, taken immediately before it swaps in the
rebuilt database.

Only the newest two snapshots for a given database file are kept — older
ones are pruned automatically before the next one is taken, so this doesn't
grow without bound.

To roll back to a snapshot, stop anything with the database open (see
[the `serve` caveat](#restart-serve-after-upgrading) below), then replace
the live file:

```bash
cp ~/.comemory/comemory.db.pre-v12.bak ~/.comemory/comemory.db
rm -f ~/.comemory/comemory.db-wal ~/.comemory/comemory.db-shm
```

The `-wal` / `-shm` sidecars belong to the file you just replaced, so remove
them too — otherwise the next open can try to replay WAL frames against a
database they don't belong to.

A snapshot is validated (`PRAGMA quick_check`) before comemory ever reuses
or trusts it, so a snapshot left behind by a killed process won't silently
stand in for a real one on your next upgrade — but that check doesn't run
retroactively on old `.bak` files sitting on disk, so verify one yourself
before relying on it for a manual restore:

```bash
sqlite3 ~/.comemory/comemory.db.pre-v12.bak "PRAGMA quick_check;"
```

## Skipping the snapshot

`COMEMORY_SKIP_MIGRATION_BACKUP=1` (or `true`) skips the pre-migration
snapshot entirely, for a database large enough that the extra `VACUUM INTO`
pass matters and you have your own backup story:

```bash
COMEMORY_SKIP_MIGRATION_BACKUP=1 comemory doctor
```

There is no size threshold that skips the snapshot automatically — the
stderr notice exists so a long pause on a large database is explained
rather than mysterious, and this variable is the deliberate opt-out for
someone who already knows the cost and wants the speed.

## Downgrading (or: running an older comemory against a newer database)

comemory has no down migrations — a schema upgrade is one-way, and the
pre-migration snapshot (or your own backup) is the recovery path, not a
reversible migration graph. If you point an *older* `comemory` at a database
a *newer* one already migrated, every command refuses with `Error::Migration`
and exits `70`, same as the forward-compat guard above.

`comemory doctor` is the one exception: since its whole job is explaining a
broken state, it doesn't also fail closed. It falls back to a read-only
connection and reports the mismatch instead of erroring out:

```bash
comemory doctor --json
# { ..., "unknown_migration_keys": ["0014_future_migration"] }
```

An empty `unknown_migration_keys` list means your build understands
everything in the database. A non-empty one means: install a comemory at
least as new as whatever last wrote this database.

## Restart `serve` after upgrading

`comemory serve` opens its database connection once, at startup, and holds
it for the life of the process. If you upgrade the `comemory` binary while a
`serve` process from the old binary is still running, that process keeps
using its already-open (pre-upgrade) connection — it does not notice, and
does not re-run preflight or the migration chain. Restart `serve` after
upgrading so it opens a fresh connection against the now-migrated schema:

```bash
# after installing the new binary
pkill -f 'comemory serve' || true
comemory serve --open
```

This is the one case cross-process coordination is genuinely out of scope
for the migration safety net — a long-running server holding a stale
connection while a separate CLI invocation migrates the file underneath it
is not something SQLite (or comemory) can detect for you.

## See also

- [Architecture: schema migration & upgrade safety](../architecture.md#32-schema-migration--upgrade-safety)
  — the mechanism, in implementation terms.
- [Configuration](../configuration.md) — `COMEMORY_SKIP_MIGRATION_BACKUP` and
  every other environment variable.
- [Prune, rebuild, and gc](prune-and-gc.md) — `comemory rebuild`, which
  shares the pre-swap snapshot mechanism described here.
- [CLI reference](../cli-reference.md) — `comemory doctor`'s full flag list.
