# `comemory backup`

Back up the data directory, restore a backup under a new stream epoch, and
merge an erasure manifest a restore could not find (#256). Nested:
`create` / `restore` / `merge-erasures`.

`create` writes a self-contained directory under `memory-save.lock`: a
`VACUUM INTO` copy of `comemory.db`, a copy of `memories/` (`.trash/`
included) and `backup.json` (`{created_at, binary_version, schema_markers,
epoch}`), written last.

`restore` never keeps the backup's stream identity. It records
`restore.pending`, copies the snapshot beside the live files and opens it (an
older snapshot migrates forward), marks it `merging`, mints a new epoch into
`replica/identity.json` (reason `restore`) and gives it this data directory's
device id, then erases every entity the erasure manifest names — in full,
without appending to the manifest — so nothing erased after the backup comes
back. It snapshots the live database to `comemory.db.pre-restore.bak`, moves
`memories/` to `memories.pre-restore/` and the restored tree into place, and
copies the staged database into the live file in place (the connections a
running `serve` holds read the new content). A rerun with the same
directory finishes a swap a killed run left pending. A snapshot whose page
size differs from the live file's is refused, naming both, before anything
changes.

When the manifest is not established (missing, torn, edited or short), the
restore completes **local-only**: the database is marked `erasure_unknown`,
every replica and legacy sync route answers `503 restore_unverified`, a
drain records `network: "restore_unverified"` and sends nothing, and
`comemory rebuild` keeps the state. `merge-erasures FILE` puts an established
manifest in place, merges it and clears the state. A peer syncing with a
restored engine sees the new epoch and rebootstraps with its pending edits
kept.

**Runnable tests:** `tests/cli__backup.rs`, `tests/replica_recovery_4.rs`,
colocated `src/domains/maintenance/tests/backup.rs`

**HTTP:** none — CLI-only (`transport: "cli-only"` in `GET /api/v1/commands`;
asserted by `tests/api__parity.rs`). A request must never replace the
database a server holds open. While a restore is unverified every sync route
answers `503 restore_unverified` — covered by
`tests/replica_recovery_4.rs::restore_without_a_manifest_fails_closed`

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

_None at the `backup` level._ Nested subcommand required: `create` |
`restore` | `merge-erasures`. `restore <DIR>` takes the backup directory;
`merge-erasures <FILE>` takes an `erasures.jsonl`.

## Flags

| Flag | Subcommand | Meaning |
| --- | --- | --- |
| `--out <DIR>` | `create` | Write the backup here instead of `<data_dir>/backups/<UTC timestamp>/` |
| `--confirm` | `restore` | Required: the restore replaces the live database and markdown |
| `--erasure-manifest <FILE>` | `restore` | Merge this manifest instead of `<data_dir>/replica/erasures.jsonl` |

## Scenarios

### backup-01 Create a backup

- **Flags:** `--out`, `--json`
- **Setup:** a saved memory
- **Command:** `comemory backup create --out <dir> --json`; `comemory backup create`
- **Expect:** `{dir, memory_files, created_at, binary_version, schema_markers,
  epoch}`; `<dir>` holds `comemory.db`, `memories/` and `backup.json`; the
  default lands under `<data_dir>/backups/<timestamp>/`; no database is exit
  69 and creates none.
- **Covered by:** `tests/cli__backup.rs::create_writes_a_backup_directory_with_its_descriptor`,
  `tests/cli__backup.rs::create_without_a_database_is_refused_and_creates_none`

### backup-02 Restore a hub: new epoch, erasures merged, peers rebootstrap (AC-9)

- **Flags:** `--confirm`
- **Setup:** a real hub; a client saves and syncs; `backup create`; more
  writes; an erase on the hub; the client keeps an unpushed edit
- **Command:** `comemory backup restore <dir> --confirm` — SIGKILLed mid-swap,
  then rerun
- **Expect:** sync answers `503 restore_unverified` while the swap is
  pending; the rerun reports `resumed: true`; a new epoch, the same device
  id, no erased content outside `*.pre-restore`; the client rebootstraps,
  pushes its edit and is caught up; a `.bak` copied over `comemory.db` by
  hand is re-epoched on its first sync read with the erasure barred again.
- **Covered by:** `tests/replica_recovery_4.rs::restore_rotates_the_epoch_and_merges_erasures`,
  `tests/cli__backup.rs::a_restore_killed_mid_swap_finishes_on_rerun`

### backup-03 Restore without a manifest fails closed until merged (AC-9)

- **Flags:** `--confirm`
- **Setup:** a real hub with an erase after its backup; the manifest moved
  away
- **Command:** `comemory backup restore <dir> --confirm`, then `comemory
  rebuild`, then `comemory backup merge-erasures <file>`
- **Expect:** `restore_state: "erasure_unknown"`; every replica and legacy
  sync route answers `503 restore_unverified`, also after the rebuild; the
  client's sync records `network: "restore_unverified"` and pushes nothing;
  `merge-erasures` clears the state, erases the entity again, and the client
  rebootstraps and catches up.
- **Covered by:** `tests/replica_recovery_4.rs::restore_without_a_manifest_fails_closed`

### backup-04 Refusals that change nothing

- **Flags:** `--confirm`
- **Command:** `comemory backup restore <dir>`; `comemory backup restore
  <dir-with-8192-byte-pages> --confirm`; `comemory backup merge-erasures
  <torn-file>`
- **Expect:** exit 70 `confirmation required`; exit 64 naming both files and
  both page sizes; exit 75 `not an established erasure manifest`, the
  manifest untouched.
- **Covered by:** `tests/cli__backup.rs::restore_without_confirm_is_refused_and_changes_nothing`,
  `tests/cli__backup.rs::a_snapshot_with_another_page_size_is_refused_naming_both`,
  `tests/cli__backup.rs::merge_erasures_refuses_a_manifest_that_is_not_established`

### backup-05 Restore with a manifest kept elsewhere

- **Flags:** `--confirm`, `--erasure-manifest`
- **Setup:** an erase after the backup; `replica/erasures.jsonl` moved away
- **Command:** `comemory backup restore <dir> --confirm --erasure-manifest <file>`
- **Expect:** the named manifest is merged: `restore_state: null`,
  `erasures_merged: 1`, the erased memory stays gone.
- **Covered by:** `tests/cli__backup.rs::restore_reads_the_named_erasure_manifest`
