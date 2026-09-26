# Upgrading comemory

**Goal:** understand what happens to `~/.comemory/comemory.db` when you install
a newer `comemory`, where the safety net lives, and how to use it if something
goes wrong.

## Getting the newer binary: `comemory upgrade`

```bash
comemory upgrade --check   # "update available: 0.18.2 → 0.19.0", or "is up to date"
comemory upgrade           # swap this binary for the newest release, in place
```

`upgrade` follows the GitHub release redirect to find the newest tag, works
out how the running binary was installed, compares, and then:

| Channel | What `upgrade` does |
| --- | --- |
| **Standalone** — `install.sh`, or a tarball you unpacked | Downloads that release's own `install.sh` and runs it pinned (`--version <tag> --dir <this binary's dir> --no-modify-path`), exporting `COMEMORY_DATA_DIR` so it ensures the right data directory's daemon. The script verifies the archive's SHA-256, runs the new binary's `--version` before touching anything, then renames it over the running one — an atomic swap, so the process you launched keeps executing the old inode until it exits. Right after the swap it runs the new binary's own `sync daemon ensure` (see [rollback](#when-the-daemon-never-comes-up) below; a failure exits 69 before anything else runs), then refreshes Bash, Zsh, Fish, and PowerShell completions idempotently |
| **Homebrew** — under a `Cellar` / Homebrew prefix | Runs `brew upgrade comemory`. The formula regenerates Homebrew-managed Bash, Zsh, Fish, and PowerShell completions. The tap can lag a GitHub release by a few minutes; the report says so instead of failing. `comemory upgrade` then runs `sync daemon ensure` itself against the linked Cellar binary — the formula's own `post_install` hook that would do this at `brew` time is pending [homebrew-tap#1](https://github.com/Falconiere/homebrew-tap/issues/1), so a bare `brew install` / `brew upgrade` alone does not yet guarantee an immediately ready daemon; running `comemory upgrade` afterward does |
| **`cargo install`** — listed in `$CARGO_HOME/.crates.toml` | Refuses (exit 64) and prints the rebuild recipe: rebuild at that tag with the finalizing wrapper, `git checkout vX.Y.Z && bash scripts/dev-install.sh`, in that checkout — it builds, installs, and ensures the daemon in one step. A bare `cargo install` recipe instead must be followed by `comemory sync daemon ensure` |

After the swap it runs the binary on disk with `--version` and fails loudly if
that is not the release it asked for. Flags: `--version <v>` pins a release
instead of the latest; `--force` allows a reinstall or a downgrade (without
it, an older target is a usage error); `--json` runs the installer quietly and
prints `{current, latest, target, channel, exe, status, hint}` with `status`
one of `up_to_date` / `available` / `upgraded` / `installed`. Exit codes: `69`
when the release host cannot be reached (the command needs `curl` or `wget`
on `PATH`), `64` for a bad `--version` or an unforced downgrade, `70` when the
installer itself failed — its last stderr lines are relayed.

There is no HTTP twin: `comemory serve` never replaces its own binary on
request (`transport: "cli-only"` in `GET /api/v1/commands`).

## The daemon `upgrade` leaves running

`upgrade` never stops at swapping the binary. In every outcome except
`--check` — a fresh `upgraded`/`installed` swap, and just as much an
`up_to_date` no-op — it also runs the **installed** binary's own `sync
daemon ensure --json` (never this, possibly just-replaced, process's) and
requires that coordinator to answer ready on that exact binary and version
before it reports success. `--check` never reaches any of this: it only
compares versions, starts nothing, probes nothing, and writes nothing.
`--data-dir` (or `COMEMORY_DATA_DIR`) picks which data directory's
coordinator gets ensured; the default data directory is never created just
to check one.

Under `--json` the report gains a `daemon` object, absent entirely when
`--check` was passed:

```json
{ "current": "0.50.0", "latest": "0.50.1", "target": "0.50.1",
  "channel": {"kind": "standalone", "dir": "/u/.local/bin"},
  "exe": "/u/.local/bin/comemory", "status": "upgraded",
  "daemon": { "ready": true, "action": "none", "supervisor": "launchd",
              "version": "0.50.1", "binary": "/u/.local/bin/comemory",
              "binary_file": "16777232:9123456", "pid": 4242,
              "data_dir": "/u/.comemory" } }
```

When that coordinator never becomes ready, `upgrade` exits **69** with
nothing on stdout:

```
error: /u/.local/bin/comemory is now 0.50.1 (binary replaced), but the sync daemon is not ready: <cause> — run: comemory sync daemon repair
```

`binary replaced` names a swap that actually happened this run; `binary
unchanged` names an already-current or `--force`-free no-op where only the
`ensure` failed. On the Standalone channel, that failure is usually
`install.sh`'s own rollback (next section), relayed here as
`Error::Unavailable`; any other `install.sh` failure keeps exit `70`.

### When the daemon never comes up (`install.sh` rollback) {#when-the-daemon-never-comes-up}

On the Standalone channel, if the post-swap `sync daemon ensure` never
reports ready — an old coordinator still stopping, a data directory the new
binary cannot write to, an `external` supervisor with nobody running `sync
daemon run` (see [Configuration](../configuration.md)) — `install.sh`
restores the exact file it replaced and reruns *that* restored file's own
`ensure` before failing:

```
error: sync daemon not ready after installing comemory v0.50.1: <cause>
    rolled back to comemory 0.50.0 at /u/.local/bin/comemory (sync daemon: ready)
```

A first-time install has no previous binary to restore, so it keeps the new
file instead and names the fix at its absolute path (PATH setup never ran):

```
error: sync daemon not ready after installing comemory v0.50.1: <cause>
    comemory v0.50.1 is installed at /u/.local/bin/comemory; fix the cause, then run: /u/.local/bin/comemory sync daemon ensure
```

Either way the process exits **69**. Rollback never opens the store, so it
cannot introduce a schema mismatch on its own; if the restored coordinator's
store reports `too_new`, that predates this install and the message adds a
forward-recovery line naming `comemory doctor`.

## Schema upgrades are automatic

There is no `comemory migrate` command. Install a newer binary, run any
command, and the schema upgrades in place on that first call — `search`,
`save`, `doctor`, whichever you happen to run next:

```bash
comemory upgrade        # or: brew upgrade comemory / cargo install --path .
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

   Migration 28 recognizes one historical exception:
   `0026_repository_approval`, written by a short-lived build before that
   table joined the released migration 25. It keeps the repository approval
   rows and repairs that build's document table to the released shape,
   retaining active revisions and removing staged or superseded revisions.
   Other unknown migration markers still cause a refusal.
2. **If any migration is pending — additive or destructive — comemory
   snapshots the whole database first**, with SQLite's `VACUUM INTO` —
   safer than a raw file copy, since it captures committed writes still
   sitting in the WAL file rather than only what's been checkpointed to the
   main file. You see one line on stderr before it starts, whether the
   pending migration merely adds a column or drops a table:

   ```
   comemory: snapshotting database to /home/you/.comemory/comemory.db.pre-v12.bak before migrating (set COMEMORY_SKIP_MIGRATION_BACKUP=1 to skip)
   ```

   Only then does the migration chain run.

Destructiveness decides what happens if that snapshot *fails*, not whether
it's attempted: when the pending upgrade includes a migration that could
destroy data (dropping a table, or rewriting rows in place), a failed
snapshot refuses the upgrade outright rather than risk it unrecoverably. When
every pending migration is purely additive, a failed snapshot only warns on
stderr and the upgrade proceeds anyway. An already-current database costs no
extra writes at all — the fast path only reads `schema_meta`.

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

A snapshot is validated (`PRAGMA quick_check`) before comemory ever reuses
or trusts it, so a snapshot left behind by a killed process won't silently
stand in for a real one on your next upgrade — but that check doesn't run
retroactively on old `.bak` files sitting on disk, so verify one yourself
before relying on it for a manual restore:

```bash
sqlite3 ~/.comemory/comemory.db.pre-v12.bak "PRAGMA quick_check;"
```

Copying one of these snapshots back over the live file by hand still works —
stop anything with the database open (see
[the `serve` caveat](#restart-serve-after-upgrading) below), replace the file,
and remove its `-wal` / `-shm` sidecars, which belong to the database you just
replaced and would otherwise try to replay WAL frames against a file they
don't belong to — but it's no longer the recommended path: a raw copy skips
identity and replication bookkeeping (see [Backup and
restore](#backup-and-restore) below).

## Backup and restore

For a restore point that carries its own safety checks — rather than a bare
file you copy by hand — take a full backup and restore it back with
`comemory backup`:

```bash
comemory backup create                    # snapshots the db and memories/
comemory backup restore <dir> --confirm   # restores one back into place
```

`backup restore` takes the same lock order every other writer does
(`memory-save.lock`, then the identity lock), migrates an older snapshot
forward through the ordinary schema-upgrade chain, and refuses outright —
naming both sizes — a snapshot whose page size doesn't match the live file's.
It mints a fresh stream epoch on every restore (a peer that already synced
past this point rebootstraps, keeping and pushing whatever it owed), and
merges the erasure manifest first, so permanently erased content can never
come back through a restore. Every sync route answers `503 restore_unverified`
until that merge completes — from a real erasure manifest, or from
`comemory backup merge-erasures FILE` when the one alongside the backup went
missing.

The swap into place happens through the same in-place SQLite backup API
`comemory rebuild` uses (see [Prune, rebuild, and
gc](prune-and-gc.md#rebuild-from-markdown)): a restore killed mid-swap leaves
the prior content in place, and simply re-running `backup restore` with the
same directory finishes it. While either is running, writers wait — up to
`[sync] pause_wait` for a CLI markdown write, 5 seconds for an HTTP or MCP
one — and then fail `busy` rather than being silently dropped; nothing is
acknowledged and left unrecorded.

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
a *newer* one already migrated, every command refuses with
`Error::SchemaTooNew` and exits `70` — this *is* the forward-compat guard
above, triggered in the direction of an older binary meeting a newer
database.

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
comemory serve
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
  shares the pre-swap snapshot mechanism described here, and `comemory erase`.
- [CLI reference](../cli-reference.md) — `comemory doctor` and `comemory
  backup`'s full flag lists.
