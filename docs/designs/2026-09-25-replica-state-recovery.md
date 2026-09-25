# Preserve replica state through bootstrap, rebuild, purge and recovery — Design

**Date:** 2026-09-25   **Status:** Approved   **Author:** Auto
**Topic:** What happens to `replica-v1` state when a database is upgraded from
a release that predates it, rebuilt, garbage-collected, permanently erased,
restored from a backup, or opened by a failed migration (issue
[#256](https://github.com/Falconiere/comemory/issues/256)).

Parent epic: Falconiere/comemory#248. Builds on
[the replica-v1 journal](2026-09-21-replica-v1-journal.md) (#250),
[memory mutation capture](2026-09-22-memory-mutation-capture.md) (#251),
[code generation replication](2026-09-22-code-generation-replication.md) (#252),
[document revision replication](2026-09-23-document-revision-replication.md) (#253),
[feedback and activity replication](2026-09-24-feedback-activity-replication.md) (#254)
and [the exchange client](2026-09-24-replica-exchange-client.md) (#255).

## Problem

The journal, the outbox and the exchange state are now the only record of what
a machine owes and what it has acknowledged. Every path that rewrites or rolls
back the database has to carry that state, and today several do not:

1. **A database from before the journal never fully joins it.** Seeding covers
   live memories only. Memories in `.trash/` are never journalled, so a restore
   has no deletion to reference and a peer never learns of the deletion.
   Documents indexed before migration 0025 are never journalled: the writer's
   fingerprint skip means an unchanged file is never re-captured, and neither
   is a document indexed while its repository was unapproved and approved
   later. On a client, seeded memories are discarded from the outbox, so a
   machine that never synced before upgrading never uploads what it wrote.
2. **Seeding races a concurrent save.** `bootstrap::seed_one` checks for a
   revision and loads the markdown outside its transaction
   (`src/domains/sync/replica/bootstrap.rs:94-110`). A save that commits
   between the two leaves the seed as the newest revision, carrying the older
   text.
3. **A rebuild forgets progress and one kind of owed upload.** `schema_meta` is
   not copied except `code_format:*`, so after a hub rebuild the manifest
   withholds every capability until a full rescan finishes. A `staged` code
   generation is dropped even when a pending outbox operation still names it
   (`src/store/rebuild_copy_code.rs:45-58`), which loses an owed code push.
   `gc_runs.activity_rows` is dropped by its copy pass.
4. **Retention can expire an event that was never delivered.** The
   `PastRetention` redaction (`src/store/replica_redaction.rs:40-51`) does not
   exclude a payload a pending outbox operation still has to upload.
5. **Nothing erases permanently.** A memory purge erases only the journal
   copies of verdicts on it, never the memory's own payload bytes; no path
   erases a document's payload; staged parts, the replay scratch and freed
   pages are never reached.
6. **Nothing supports a restore.** There is no backup or restore command, the
   documented procedure is copying a `.bak` file over `comemory.db`
   (`docs/guides/upgrading.md:101-112`), the epoch never rotates, and nothing
   outside the database remembers what was erased.
7. **Revoked pulled code still shows.** Pulled document passages are hidden
   unless approved; `repos` and the code graph read pulled generations with no
   approval check (`src/store/remote_code_view.rs:41-46,115-128`,
   `src/store/code_graph_edges.rs:18-30`).
8. **A rebuild races every other process and can damage the new file.** It
   takes no lock, so a write another process commits to the old file after the
   copy is lost when the rename lands. Worse, when the last connection to a
   renamed-away file closes, SQLite checkpoints it and deletes `<path>-wal`
   and `<path>-shm` by name — the new file's — so a stale CLI process or the
   server's swapped-out connection can discard committed frames of the new
   database.
9. **A failed migration can look healthy.** Daemon status reports only whether
   the OS unit runs (`src/domains/sync/daemon_unit.rs:221-258`).

## Non-Goals

1. **No re-publication of acknowledged operations a restored upstream lost.**
   A restored upstream rotates its epoch and every client rebootstraps with its
   pending operations intact (#255); operations the upstream acknowledged and
   then lost are reported by `verify` as the residual difference, not resent.
   Re-sending them could order old writes after newer ones the restored
   upstream kept. (#255 deferred the question here; decided against, Jev 0.16.)
2. **No generic sequence compaction.** No feed row is ever deleted.
3. **No account or workspace deletion semantics.** `comemory erase` is the
   engine primitive CodaSignal/comemory.io#80 can call; what the product erases
   and when stays there.
4. **No new wire operation.** Erasure is not replicated as its own operation.
   Peers learn of the deletion through the ordinary tombstone; the engine that
   erased refuses any later offer of the erased bytes.
5. **No daemon lifecycle work.** `src/domains/sync/daemon.rs` and `watch.rs`
   are not modified; #257 replaces both with one resident coordinator. This
   issue exposes the exchange pause and the store health as two narrow
   interfaces (§ Lock order, § Store health).
6. **No downgrade tooling.** An old executable keeps refusing a newer database.
7. **No erase for code generations.** They carry no source text.
8. **No change to the legacy wire's shapes.** Legacy routes gain only the
   `503 restore_unverified` refusal; seeded tombstones are journalled on the
   replica feed only, so no legacy delete entry is added for them.
9. **`gc`'s trash purge stays housekeeping.** It removes the local trash file
   and mirror rows, and erases verdict copies as today, but leaves the memory's
   own journal payload: every engine purges its trash on its own schedule, and
   turning that into a permanent barrier would refuse a peer's later
   legitimate restore (Jev 0.96). Only `comemory erase` is permanent.

## Architecture

### Seeding: every eligible entity, once

`domains::sync::replica::bootstrap` becomes one bounded, resumable walk over
three scans. Each resumes from its own `schema_meta` cursor and is idempotent by
row state, so a lost cursor costs a rescan, never a second seed:

| Scan | Selects | Journals |
| --- | --- | --- |
| live memories | `memories.deleted_at IS NULL` with no `replica_revision`, ascending id | upsert through `journal::record_write` (as today) |
| trashed memories | `.trash/*.md` files on disk, by id (a rebuild replays only top-level markdown, so the mirror cannot be the source), with no revision | tombstone on the replica feed only (`replica_journal::append` plus the same outbox rule as the other seeds, no `sync_log` row) — the payload-free revision a later restore references |
| documents | `documents` with no `document_share` row whose source file still hashes to `documents.revision_hash` | the revision `domains::documents::journal::record_revision` captures at index time, extracted from the file |

The document scan runs again from the start whenever the approval map's
fingerprint differs from the one recorded when it last completed
(`replica_seed_documents_policy`), because `record_revision` withholds a
document whose label has no approved canonical name or no repository root and
records nothing for it. A document whose file changed or vanished is left to
the next `comemory index`. A purged memory is gone from disk and is not seeded.
Documents on an engine with no policy (a hub) stay withheld, as they are
today; seeding them is a client concern.

**The race.** Each seed loads its source (markdown, or file plus extraction)
outside any transaction, then opens `BEGIN IMMEDIATE`, re-checks the revision
(and, for a document, the file hash) inside it, and journals only if nothing
changed. A save that committed first leaves a revision and the seed is
skipped; a save that commits after waits on the write lock and journals on top
of the seed. The walk exposes `load` and `commit` halves so a test can put a
real save between them.

**Ids.** A seed's operation id is minted inside the transaction that commits
it, so a killed walk leaves no id behind. Event ids are stamped on their rows
in the capture transaction (#254), which skips a row that already carries one.

**Where it runs.** The hub advances seeding in the replica cores, as today. A
client advances it in the drain's adoption step, before events: `Reach::All`
seeds to completion, the inline push seeds one batch. On an engine that has a
`replica-v1` upstream (`sync_exchange::has_replica_upstream`) seeding keeps the
outbox row it writes; on any other engine it discards it, as #255 decided.

**Adopting earlier seeds.** Memories seeded while the engine was not a client
— including every v0.48 client that already upgraded — have no outbox row. On
every pass whose key has a cursor at the captured head and no replay in
progress, the drain enqueues each local-origin memory seed position whose entity has no
outbox row in any state and no `replica_binding` under the key. An entity the
upstream already holds was bound when its entry was pulled, so it is never
sent twice; one it lacks is sent once; one with any later local operation is
carried by that operation and never overtaken by its older seed. Adoption runs
before the push leg of the same pass, so a never-synced client's history goes
out on its first `comemory sync`. The marker
`replica_seed_adopted:<api_url>:<workspace>` records the last seed position
adopted for the key, so a pass only reads positions above it.

**Code generations** are not seeded: one is captured at push time from the
current index. **Feedback and activity** keep the #254 backfill, which sees
only rows still inside retention.

### Rebuild keeps progress and owed uploads

- Every `schema_meta` key with the prefix `replica_` is copied (seeding
  cursors, capture and backfill cursors, event adoption, seed adoption, the
  document policy fingerprint, and `replica_restore_state`).
- After the copy, one query counts live memories with no revision. If any
  exist — a markdown file placed in `memories/` by hand, or a memory the old
  journal never saw — the live-memory scan restarts from the beginning, so the
  manifest withholds capabilities until each is seeded once. Otherwise a
  finished hub keeps advertising its capabilities through the rebuild.
- `code_generation` rows in state `staged` that a `pending` operation names are
  copied with their `remote_code_*` projection rows.
- `gc_runs` copies every column.

### Garbage collection

- The retention redaction skips every payload a `pending` outbox operation
  names. An event that has not reached its upstream keeps its bytes past
  retention; its materialized row is still evicted locally.
- Every redaction — expiry and erasure — also blanks the payload inside each
  `replica_replay.entry_json` naming the digest (the scratch row keeps its
  position, now with `payload_state` `expired` or `erased`), and deletes every
  complete staging set whose assembled bytes hash to the digest. An incomplete
  set cannot be attributed; the 24-hour stage sweep removes it.
- `gc_runs` gains `staged_rows`, which `record_run` writes; `gc --json`
  already reports it, the text line gains it, and the console's `last_run`
  shows it.

### Permanent erase

One core, `domains::maintenance::erase`, erases one entity. The command and
route take `memory-save.lock`, then `identity.lock`, then call the lock-free
`erase::apply_locked`, which restore and `ensure` call under the locks they
already hold.

| Entity | Removes | Keeps |
| --- | --- | --- |
| memory | markdown (live and `.trash/`), the mirror row and tags, FTS, substring, vector, code refs, edges, feedback counters and verdicts; candidate-observation passages (redacted as a purge redacts them); the title in the summaries of `activity_log` rows naming the memory; the bytes of every payload the entity's feed rows name, of the verdict events on it and of those activity events | feed rows, revisions, receipts, digests, operation ids; the queries users typed (`retrieval_log`, candidate query observations), which are their own input, not the memory's text |
| local document | `documents`, `document_chunks`, `document_fts`, `source_files` row, links and edges, `document_share`; the bytes of every payload the entity's feed rows name | the source file, feed rows, revisions, receipts |
| pulled document | `remote_document*` rows; the bytes of every payload the entity's feed rows name | feed rows, revisions, receipts |

In one transaction, with `PRAGMA secure_delete = ON` on its connection, erase:

1. journals a tombstone first when the entity is live and this engine can
   delete it through the ordinary path — a memory (memory deletion is
   workspace-wide by design), or a document this engine shares. A pulled
   document is erased locally only: journalling its tombstone would delete it
   for the whole workspace;
2. deletes the rows above and blanks the payload bytes (`redaction =
   erased`);
3. blanks the entity's replay scratch and deletes its complete staging sets;
4. marks every `pending` upsert or restore for the entity `rejected` with
   disposition `payload_erased` (a pending tombstone still goes).

After the commit it runs `optimize` on every FTS index it touched, so deleted
postings leave the index segments, and `wal_checkpoint(TRUNCATE)`, retried for
up to 5 s while a reader holds an open transaction and reported as
`wal_truncated` either way. It refreshes the derived graph state as other
post-write steps do.

**The barrier** is the erased digest (Jev 0.88 over refusing the whole entity
key): the engine never stores bytes for an erased digest again
(`store_payload` is already `INSERT OR IGNORE`). An import or a pulled entry
carrying it — a replay of the original operation, or a peer's restore, which
carries the deleted revision's bytes — is `payload_erased` and materializes
nothing. A re-index of an erased local document's unchanged file records its
share blocked `erased` and journals nothing. New content under the same key is
a new revision and flows normally; that includes saving the erased text again,
because a memory payload carries its `created` time and a new save gets a new
one — erasure removes what exists, it does not forbid writing it again. An
ordinary tombstone stays restorable.

**What remains.** Rollback snapshots are backups, not live copies:
`comemory.db.pre-v*.bak`, `comemory.db.pre-rebuild.bak`,
`comemory.db.pre-restore.bak`, `memories.pre-restore/` and `backup create`
directories still hold pre-erase state. Erase names the ones present in the
data directory in its report (`snapshots_with_prior_state`); restoring any of
them through `comemory backup restore` merges the erasure manifest, so the
supported tooling cannot bring erased content back. Deleting them is the
operator's decision.

`comemory erase --memory <id>` / `--document <shared-id>` and
`POST /api/v1/erase` (mutating, confirm-gated) run the core. An entity this
engine never held is `404 not_found` and nothing is written.

### Stream identity and the erasure manifest live outside the corpus

Two files under `<data_dir>/replica/`, never part of a backup and never
replaced by a restore, written under `<data_dir>/replica/identity.lock`
(`utilities::file_lock`):

- `identity.json` — `{v, epoch, device_id, epochs: [{epoch, since, reason}],
  erasures}`: the current stream epoch, the device id, the epoch history and
  the manifest's line count. Written to a temporary file, fsynced, renamed,
  and the directory fsynced.
- `erasures.jsonl` — append-only, one line per erased entity
  `{v, kind, key, digests, erased_at, prev}`, where `prev` is the SHA-256 of the
  previous line, so a truncation or an edit breaks the chain. Each erase
  appends and fsyncs its line, then advances the count, then commits. A crash
  after the append leaves the manifest one line ahead, which only means one
  more entity is erased on the next merge.

Both are created, under the lock, by the first replica read or drain that finds
them absent, from the database: its epoch, its device id and every entity
whose payload is already erased. The manifest is **established** when the file
exists, every line parses, the chain verifies, and it holds at least the count
`identity.json` names.

`replica::identity::ensure` runs before every replica core and every drain.
It reads the epochs under `identity.lock`; on a mismatch — the database was
replaced by something other than the supported restore, such as a `.bak`
copied over it by hand — it releases that lock, takes `memory-save.lock` and
then `identity.lock` (§ Lock order), re-checks, and runs the restore steps 3
and 4 below in place, so stream identity is never retained through a manual
copy.

### Backup and restore

`comemory backup create [--out DIR]` (default
`<data_dir>/backups/<UTC timestamp>/`) writes, under the exchange gate: a
`VACUUM INTO` copy of the database, a copy of `memories/` including `.trash/`,
and `backup.json` (`{created_at, binary_version, schema_markers, epoch}`).

`comemory backup restore DIR --confirm [--erasure-manifest FILE]`:

1. Takes the exchange gate and `memory-save.lock`; writes `restore.pending`
   naming the staged paths.
2. Copies the snapshot's database to `comemory.db.restore.tmp` and opens it, so
   an older snapshot is migrated forward (with its own pre-migration snapshot);
   refuses, naming both, a snapshot whose page size differs from the live
   file's. Copies the snapshot's `memories/` to `memories.restore/`.
3. Sets `replica_restore_state = merging`; mints a new epoch and records it in
   `identity.json` with reason `restore`; writes the identity's device id into
   the staged database.
4. When the manifest is established (`--erasure-manifest` overrides the
   default path), runs `erase` for every entry against the staged database and
   `memories.restore/` without appending to the manifest, then clears the
   restore state. When it cannot be established, sets the state to
   `erasure_unknown` and goes on: the restore is local-only (Jev 0.95 over
   refusing outright).
5. Snapshots the live database to `comemory.db.pre-restore.bak`, moves
   `memories/` to `memories.pre-restore/` and `memories.restore/` into its
   place, and copies the staged database into the live file in place (§ below).
   Then removes `restore.pending`.

A rerun of `comemory backup restore` with the same directory finishes a swap it
finds pending. While `replica_restore_state` is set or `restore.pending`
exists, every replica route (`changes`, `manifest`, `events`, `import`,
`stage`, `activate`) and every legacy sync route answers `503` with code
`restore_unverified`, the drain records `network: "restore_unverified"` and
sends nothing, and `comemory rebuild` keeps the state (it is a `replica_`
key). `comemory backup merge-erasures FILE` copies `FILE` into place when it is
established, merges it and clears the state. `comemory doctor` reports the
identity, the manifest and the restore state.

A client whose upstream was restored sees the new epoch on its next request and
rebootstraps with every pending operation kept (#255). A restored engine that
is itself a client resends its snapshot's pending operations under their
original ids; the upstream answers those it already holds `duplicate` from its
receipts.

### Policy-revoked remote-only caches

An engine that has loaded a policy snapshot (a `sync_policy_snapshot` row
exists) shows a repository's pulled code generation in `repos` and the code
graph only while `repository_approval` approves it, as pulled document passages
already are. An engine that never loaded a policy — a hub, a standalone
`serve` — keeps today's behavior (Jev 0.93 over always requiring approval).
The cache is hidden, not deleted, so reapproval shows it again without a
reload, and entries pulled while revoked are held and applied when the holds
come due (#255). Local originals, local indexes and pulled memories are
untouched. Candidate observations are never journalled; a test asserts that
none of their text reaches a replica table or a push body, and their purge
keeps redacting them.

### Lock order

Outermost first; a holder never takes an earlier one:

1. **Exchange gate** — `sync.lock` today. Taken by every exchange pass and by
   rebuild, restore and `backup create`. The inline push only tries it.
2. **`memory-save.lock`** — taken by rebuild and restore for their whole run,
   and by every writer that moves or rewrites markdown. The rule is enforced by
   the compiler: every `MemoryStore` write (`save`, `rewrite`, the move to and
   from `.trash/`, the trash purge) takes a `&SaveGuard`, and only
   `memories::save_lock::acquire_within` returns one. Every entry point
   therefore holds it — save, `update` (body and frontmatter-only patches),
   delete, restore, `prune --apply`, `refresh_refs`, the legacy exchange
   import, `replica::materialize` (import and pull), `gc`'s trash purge,
   `erase`, and `recover::reconcile` at CLI, `serve` and MCP startup (which
   swaps its unbounded `FileLock::acquire` for the bounded one) — taking it once, before opening its write transaction, and passing
   the guard down; no inner core acquires it again (`flock` on a fresh
   descriptor would block on the process's own hold). A server-side writer (an
   HTTP route, an MCP tool, a hub import) waits at most 5 s and then fails
   `busy` (`503 busy`); a CLI writer waits at most `[sync] pause_wait`.
   `utilities::file_lock` gains the bounded `acquire_within`.
3. **Identity lock** — `replica/identity.lock`, by erase, `ensure` and restore.
4. **Open lock** — `<path>.open.lock`, held by `connection::open`.
5. **Migration snapshot lock** — nested inside the open lock (today).
6. **SQLite write lock** — every writer, bounded by `busy_timeout` (5 s); the
   backup that replaces the content holds it from its first step to its last.

The exchange gate is reached only through `domains::sync::exchange_gate`:

```rust
pub struct ExchangePause { /* the held gate; released on drop */ }
pub fn pause(paths: &Paths, wait: Duration) -> Result<ExchangePause>; // Error::Busy after `wait`
```

It is a bounded try-acquire of `sync.lock` until #257 lands; the coordinator
then implements `pause` as pause-and-drain with the same signature, and resumes
when the guard drops. `[sync] pause_wait` (default `"60s"`) bounds it.

### Rebuild and restore replace the content in place

No file is renamed over `comemory.db` any more. New content is copied into the
live file through SQLite's online backup API, which holds one write transaction
on the destination from its first step until it is done, and applies edits made
through the backup's own source connection without restarting
(`rusqlite` feature `backup`, 0.40):

1. Pause the exchange; take `memory-save.lock`.
2. Snapshot the live file to `comemory.db.pre-rebuild.bak` (`VACUUM INTO`,
   today).
3. Create `comemory.db.rebuild.tmp` at the live file's page size
   (`store::connection::open_at_page_size`; a WAL destination refuses another
   size) and open a backup from it into the live file. Take the first step with
   zero pages, retrying `busy`/`locked` until `pause_wait`: the live file's
   write lock is now held, so no writer can commit, and since every markdown
   writer needs the `SaveGuard` the rebuild holds, no markdown moves.
4. Through the backup's source connection, replay markdown and sources into the
   temp file and copy the preserved tables from the live file into it (the
   `rebuild_copy_*` passes, reading the live file through `ATTACH`, which a WAL
   reader may do while the write lock is held;
   `copy_preserved_tables_from_old` takes `&Connection`).
5. Step to completion: every page is written in the one held transaction and
   committed at once. Checkpoint, remove the temp file, release everything.

`store::replace_in_place::run(live: &mut Connection, source: &Connection,
fill: impl FnOnce(&Connection) -> Result<()>) -> Result<()>` owns steps 3–5;
the rebuild and restore domains pass `fill`. Because the backup borrows the
source connection, `fill` opens its transactions with
`unchecked_transaction`, and the derived-state refresh
(`refresh_derived_best_effort`, which takes `&mut Connection`) runs on the live
connection after step 5.

Every existing connection — the server's shared one, an MCP call, a CLI
process, a pass waiting on the gate — sees the new content on its next
transaction; nothing reopens and no file is abandoned. A writer that waited
past `busy_timeout` fails `busy` having acknowledged nothing; one that got the
lock after step 5 writes into the new content. A process killed before step 5
commits leaves the old content; after, the new. For its duration the rebuild
is an outage for writers: hub imports, HTTP routes and MCP tools wait up to
5 s (on `memory-save.lock` or the SQLite write lock) and then answer
`503 busy`; CLI markdown writers wait up to `pause_wait` on
`memory-save.lock` and other CLI writers fail `busy` after 5 s; and the WAL
grows to about the database's size until the checkpoint. A full disk fails the
step with `SQLITE_FULL` and leaves the old content. The server's post-rebuild connection swap is removed.

### Store health

```rust
pub enum StoreHealth { Ok, SchemaTooNew { detail: String }, MigrationPending,
                       MigrationFailed { detail: String, at: String },
                       Unavailable { detail: String }, RestoreUnverified }
pub fn probe(paths: &Paths) -> StoreHealth;        // domains::maintenance::store_health
pub fn open_recorded(paths: &Paths) -> Result<Connection>;
```

`open_recorded` is `connection::open` that records a failed migration in
`<data_dir>/store-health.json` (`{state, detail, at, binary_version}`) and
removes the file on success; long-lived entry points (`serve`, the resident
coordinator) open through it. `probe` never creates or migrates: it runs the
preflight classification read-only, as `doctor` does — an unknown applied
marker is `SchemaTooNew`, pending markers are `MigrationFailed` when the record
names this binary's version and `MigrationPending` otherwise (a read-only data
directory cannot hold the record, and still reads unhealthy), a set restore
state is `RestoreUnverified`. Every state but `Ok` is unhealthy — including
`MigrationPending` right after a binary upgrade, until the first open applies
the chain.

At delivery, whatever reports daemon health reports the probe: `comemory sync
daemon status` (text and `--json`: `store`, `healthy`) and, once #257 has
merged, its coordinator health endpoint in place of that. `comemory doctor`
reports it too. Forward-version refusal and the pre-migration `VACUUM INTO`
snapshot are unchanged, and an old executable still refuses a migrated
database.

## Interfaces / Schema

### Migration `0029_replica_recovery` (additive)

| Table | Column | Holds |
| --- | --- | --- |
| `gc_runs` | `staged_rows INTEGER NOT NULL DEFAULT 0` | Orphan staged parts and staged generations swept |

Added to the `gc_runs` rebuild copy pass.

### `schema_meta` keys (all copied by rebuild)

| Key | Holds |
| --- | --- |
| `replica_seed_trash_through`, `replica_seed_documents_through` | Resume points of the two new scans |
| `replica_seed_documents_policy` | Approval fingerprint the document scan last completed under |
| `replica_seed_adopted:<api_url>:<workspace>` | Last seed position adopted for the key |
| `replica_restore_state` | `merging` or `erasure_unknown`; absent when sync reads are allowed |

### Files

| Path | Role |
| --- | --- |
| `<data_dir>/replica/identity.json`, `identity.lock` | Stream identity; its lock |
| `<data_dir>/replica/erasures.jsonl` | The erasure manifest |
| `<data_dir>/restore.pending` | Staged paths while a restore swaps |
| `<data_dir>/store-health.json` | The last failed migration, removed on success |
| `<data_dir>/backups/<timestamp>/` | Default `backup create` output |
| `comemory.db.pre-restore.bak`, `memories.pre-restore/` | What the last restore replaced |

### Commands, routes, errors, config

| Surface | Class |
| --- | --- |
| `comemory backup create [--out DIR] [--json]` | CLI-only |
| `comemory backup restore DIR --confirm [--erasure-manifest FILE] [--json]` | CLI-only |
| `comemory backup merge-erasures FILE [--json]` | CLI-only |
| `comemory erase (--memory ID \| --document SHARED_ID) --confirm [--json]` | `POST /api/v1/erase` `{memory \| document, confirm}`, mutating, confirm-gated |
| `comemory gc` | text line gains `staged_rows` |
| `comemory sync daemon status` | gains `store` and `healthy` |

`erase --json`: `{kind, key, tombstoned, payloads_erased, operations_withdrawn,
staged_removed, replay_blanked, wal_truncated, snapshots_with_prior_state}`.

Errors: `Error::Busy(String)` — gate not granted within `pause_wait`, or the
first backup step not granted — exit `75` (`EX_TEMPFAIL`), HTTP `503 busy`
through `utilities::error_code::classify`. `Error::RestoreUnverified` — exit
`75`, HTTP `503 restore_unverified`. The drain's `network` gains
`restore_unverified`.

Config: `[sync] pause_wait`, default `"60s"`. Cargo: `rusqlite` gains the
`backup` feature.

## Failure modes and edge cases

| Input / failure | Behavior |
| --- | --- |
| Legacy database opened by this binary | Pre-migration snapshot, migrations `0022`…`0029`; seeding on the first replica read or drain |
| Opened a second time | No second snapshot, no second seed |
| Seeding killed mid-batch (`SIGKILL`) | Committed seeds stay; the rest resume from row state; nothing seeded twice, no event given a second id |
| Save commits between a seed's load and commit | Seed skipped; the save's revision stands |
| Trashed memory, then `comemory rebuild`, then seeding | Seeded from `.trash/` on disk |
| Pre-journal document whose file changed | Not seeded; the next `comemory index` journals the new revision |
| Document withheld at seed time, repository approved later | Approval fingerprint changes; the scan reruns and seeds it once |
| v0.48 client that upgraded before this change | Its unbound seeds are adopted and sent once; bound ones never |
| Rebuild of a finished hub | Capabilities still advertised; no rescan |
| Rebuild with an unjournalled markdown file | Live-memory scan restarts; capabilities withheld until seeded once |
| Rebuild with a pending code push | Staged generation and projection kept; the push still goes |
| Undelivered event past retention | Bytes kept until delivered; local row evicted |
| Expired event replayed to a bootstrapping client | `payload_expired`; the client advances and counts nothing |
| Erase of a live memory | Tombstone journalled, then erased |
| Erase of a pulled document | Local cache erased; no tombstone; peers unaffected |
| Erase of an entity never held | `404 not_found`; nothing written |
| Replay of an erased digest, or a restore of an erased memory | `payload_erased`; nothing materialized |
| New content under an erased key | Accepted as a new revision |
| Unchanged re-index of an erased local document | Indexed locally; share blocked `erased`; nothing journalled |
| The erased text saved again | A new memory revision (new `created`), shared normally |
| Server-side writer waits on `memory-save.lock` past 5 s | `503 busy`; nothing acknowledged |
| Ordinary delete then restore | Restored (unchanged) |
| Restore with no established manifest | Local-only; every sync route `503 restore_unverified` until `merge-erasures` |
| `comemory rebuild` while a restore is unverified | State kept; still `restore_unverified` |
| Manifest truncated or edited | Not established; same as missing |
| Two processes erase at once | Serialized on the identity lock; the chain stays whole |
| `.bak` copied over `comemory.db` by hand | Next replica read or drain: new epoch and merge, or `restore_unverified` |
| Restore killed mid-swap | `restore.pending` remains; sync refuses; rerun finishes |
| Snapshot with another page size | Restore refused, both sizes named; nothing changed |
| Upstream restored behind a client's cursor | Client rebootstraps; pending edits kept and pushed |
| Gate held past `pause_wait` | Rebuild/restore fail `busy`; nothing changed |
| Writer waits on a rebuild past `busy_timeout` | `busy`, nothing acknowledged; a retry lands in the new content |
| Rebuild killed before the backup completes | Old content; the temp file is replaced by the next run |
| Disk full during the page copy | `SQLITE_FULL`; old content |
| Old executable opens a migrated database | Refused, file unchanged |
| Database carrying the historical `0026_repository_approval` marker (#297) | Upgrades through `0028_historical_document_revision`'s repair like any other; `store_health::probe` and the forward refusal read the same applied-marker set (`preflight::applied_keys`), so the alias never reads as `SchemaTooNew` |
| Migration with the data directory read-only | Actionable error, no new marker; probe `MigrationPending` |
| Migration killed | Each migration is its own transaction; the next open finishes |
| Coordinator or `serve` cannot open or migrate | `store-health.json` written; probe unhealthy; never reported healthy |

## Acceptance criteria

- **AC-1:** (B-1) A data directory made by the pinned `v0.43.2` binary —
  memories saved from `docs/guides/*.md` (over two seed batches), some deleted
  to `.trash/`, real `find` + `feedback --used` verdicts and the activity those
  runs recorded — upgraded by this binary and opened twice has exactly one
  pre-migration snapshot, which `v0.43.2` still opens. Served as a hub, its
  manifest advertises capabilities only when seeding finishes, and its feed
  then holds one upsert per live memory, one tombstone per trashed memory and
  one position per retained verdict and run, with no event id twice.
- **AC-2:** (B-1) Seeding the AC-1 directory with the engine killed by
  `SIGKILL` during its second and third seed calls, then restarted, ends in the
  same feed counts as an uninterrupted run, with no entity seeded twice and
  every `event_id` stamped before the first kill unchanged.
- **AC-3:** (B-1) A real save of a memory that commits between a seed's load
  and commit halves leaves that save's revision current and the seed skipped;
  CLI saves and patches run in a second process while an engine seeds leave
  every memory's revision digest equal to its markdown's payload digest.
- **AC-4:** (B-1) A `v0.43.2` data directory that also indexed a git checkout of
  `docs/guides` with `index-code` and as a labelled document source, as a
  client that never synced, with a policy snapshot approving that label,
  logged into a fresh hub: one `comemory sync` delivers every live memory,
  trashed-memory tombstone, document, verdict and run to the hub once, and a
  second sync on each side changes no count. A document seeded while its
  label was unapproved is delivered once after approval. A client that
  upgraded and caught up before this change delivers its unbound seeds once
  and none the hub already held.
- **AC-5:** (B-2) On a client holding pending, held, retryable and rejected
  operations, a pending code push, receipts, a cursor and anchor, bindings, an
  epoch and an active pulled code generation and document, `comemory rebuild`
  then `comemory index` of its local sources leaves every replica table row and
  every `replica_` key identical and every pulled row present; the next sync
  uploads the pending operations. A rebuilt hub keeps advertising its
  capabilities; one given an extra unjournalled markdown file withholds them
  until it is seeded once. `gc_runs` rows keep every column.
- **AC-6:** (B-6) An event queued while the hub is stopped and aged past
  retention keeps its bytes through `comemory gc` and is counted once by the
  hub after it restarts. A delivered event past retention is expired in
  `replica_payload`, in the replay scratch and in a complete staged set; a
  fresh client bootstrapping from that hub advances past it with
  `payload_expired` and counts nothing. Staged parts older than 24 hours are
  swept and reported by `gc` and in `gc_runs.staged_rows`, while a staged
  generation with a pending operation and every active generation survive. No
  feed row is removed.
- **AC-7:** (B-5) `comemory erase --memory` on a hub whose memory carries a
  unique token leaves that token in no file under the data directory except the
  snapshots erase names in `snapshots_with_prior_state`, keeps the memory's
  feed rows, revision and receipts, withdraws its pending upserts, and answers
  a replay of the original import envelope and a peer's restore
  `payload_erased`, while a peer pulling the hub's feed again applies nothing.
  A different memory deleted and restored the ordinary way is live, and a
  trashed memory purged by `gc` stays restorable on a peer.
- **AC-8:** (B-5) `comemory erase --document` of a shared document on its
  origin, and of the pulled copy on a peer, leaves none of its chunk text in any
  table or journal copy on either; a later pull or replay of the erased
  revision applies nothing; the origin's source file is untouched; re-indexing
  it unchanged records the share blocked `erased`; editing it shares a new
  revision.
- **AC-9:** (B-4) A hub backed up with `comemory backup create`, then written
  to and erased, then restored with `backup restore --confirm`, has a new
  epoch, keeps its device id, resurrects no erased content, and answers sync
  only after the merge. A client with pending edits and a cursor beyond the
  restored head rebootstraps on its next sync, keeps and pushes its edits and
  catches up. Restoring with the manifest removed restores locally, answers
  every sync route `503 restore_unverified` through a `comemory rebuild`, and
  holds the client's exchange until `backup merge-erasures` is given the
  manifest. A `.bak` copied over `comemory.db` by hand gets a new epoch and
  the merge on its first sync read. A restore killed mid-swap finishes on
  rerun.
- **AC-10:** (B-7) On a client with a policy snapshot, a pulled code generation
  and a pulled document of repository R: revoking R hides the generation from
  `repos` and `graph` and the document from `find --only document`, while R's
  own local index is still searched; approving R again shows them, and entries
  pulled while revoked are applied. A peer with no policy still shows a pulled
  generation. Captured candidate observations appear in no replica table and no
  push body, before or after a purge redacts them.
- **AC-11:** (B-3) While a real `comemory serve` takes HTTP writes in a loop, an
  MCP session saves, and a second process holds the exchange gate,
  `comemory rebuild` waits for the gate, then rebuilds in place: every write any
  process reported successful is in the rebuilt database with its journal row,
  every write that did not land failed `busy`, and the server's and the MCP
  session's next requests read the rebuilt content. With the gate held past
  `pause_wait`, rebuild fails `busy` and changes nothing. A rebuild killed at
  random points (20 runs) always leaves a database that opens, passes
  `integrity_check` and holds either the old or the new state.
- **AC-12:** (B-8) `v0.43.2` refuses the upgraded database and leaves its bytes
  unchanged; an upgrade with the data directory read-only fails with an
  actionable error and no new marker; an upgrade killed mid-chain completes on
  the next open. `store_health::probe` reports `SchemaTooNew`,
  `MigrationPending`, `MigrationFailed` and `RestoreUnverified` for those states
  and `Ok` otherwise, and `comemory sync daemon status --json` (the coordinator's
  health endpoint once #257 has merged) reports `healthy: false` with that
  state whenever it is not `Ok`.
- **AC-13:** (docs, CI) `scripts/replication/coverage.json` maps `B-1`…`B-8` to
  case `recovery`; `bash scripts/test-replication-e2e.sh --case recovery` runs
  `--test replica_recovery` and its siblings; the runbooks below are updated.

## Acceptance evidence

All live checks drive real spawned `comemory serve` engines and real CLI
processes over real SQLite and real HTTP, through #255's fault proxy where a
failure is induced. The legacy engine is the `v0.43.2` release asset for the
host target (`aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`,
`aarch64-unknown-linux-gnu`; CI is `x86_64` Linux), downloaded once into
`target/legacy-engine/`, checked against SHA-256 values committed in
`scripts/replication/legacy-engine.json`, and required to print
`comemory 0.43.2`. Another host, a missing asset or no network fails the test
with the download command; it never skips (Jev 1.00 over a committed database
or a source build). Interruption is a real `SIGKILL`. Time passes the way
#254's suite passes it: stored timestamps and `.trash/` mtimes moved back
(`tests/common/replica_events_support.rs`), never a mocked clock. Revocation is
a policy snapshot written through the store API a policy load uses, as #255
does. The gate is held by a real second process.

| AC | Real input | Expected result | Boundary | Check |
| --- | --- | --- | --- | --- |
| AC-1 | `v0.43.2` dir: 450 memories from `docs/guides/*.md`, 20 trashed, `find` + `feedback --used`, activity | one snapshot, openable by `v0.43.2`; per-kind feed counts equal the corpus | second open changes nothing | `replica_recovery` `legacy_database_upgrades_and_seeds_once` |
| AC-2 | AC-1 dir, `SIGKILL` twice mid-seed | counts equal the uninterrupted run; event ids unchanged | kill during a batch commit | `replica_recovery` `interrupted_seeding_resumes_without_duplicates` |
| AC-3 | Real save between `load`/`commit`; parallel CLI saves during seeding | save's revision current; digests match markdown | seed after the save's commit | `replica/tests/bootstrap.rs` `a_save_between_load_and_commit_wins` + `replica_recovery` `seeding_under_concurrent_writes_keeps_the_newest_text` |
| AC-4 | `v0.43.2` dir with a `docs/guides` git checkout code- and document-indexed; approving snapshot; fresh hub; a label approved late; a caught-up pre-change client | each once at the hub; second syncs change nothing; late-approved document once; only unbound seeds sent | trashed memory arrives as a tombstone | `replica_recovery_2` `a_legacy_client_uploads_its_history_once` + `late_approval_and_early_upgraders_send_once` |
| AC-5 | Client with every replica state, a pending code push, pulled code + document; hub with 450 seeded memories | table dumps (taken before any sync) equal; pulled rows present; pending ops upload; hub advertises | unjournalled markdown file restarts the scan | `replica_recovery_2` `rebuild_keeps_replica_state_and_pulled_caches` + `maintenance/tests/rebuild.rs` |
| AC-6 | Event queued with the hub stopped; `at` and staged `created_at` aged past retention | undelivered bytes kept then counted once; expired copies blanked; sweep reported and persisted | active generation untouched; feed count unchanged | `replica_recovery_3` `gc_keeps_what_is_owed_and_expires_the_rest` |
| AC-7 | Memory with a unique token (never used as a query) saved, found, given a verdict, shared, then erased on the hub with the report's `wal_truncated` asserted true; a peer; a trashed memory `gc` purges | token only in the named snapshots; `payload_erased` on the replayed import and the restore; re-pull applies nothing | ordinary restore works; gc purge stays restorable | `replica_recovery_3` `erase_leaves_only_the_barrier` |
| AC-8 | Shared `docs/guides` document erased on origin and peer | chunk text absent; nothing re-applied; source intact; unchanged re-index blocked `erased`; edit shared | — | `replica_recovery_3` `erased_documents_stay_erased` |
| AC-9 | Hub backup, writes, erase, restore; client with pending edits; manifest removed; `.bak` copied by hand; `SIGKILL` mid-swap | new epoch; no resurrection; client rebootstraps and pushes; `restore_unverified` through a rebuild until merge; manual copy re-epoched; rerun finishes | page-size mismatch refused | `replica_recovery_4` `restore_rotates_the_epoch_and_merges_erasures` + `restore_without_a_manifest_fails_closed` |
| AC-10 | Client with snapshot, pulled code generation + document of R, own local index of R; a no-policy peer; candidate capture + purge | hidden while revoked, local still searched, shown after approval, held entries applied; no-policy peer unchanged; no candidate text in any replica row or push | — | `replica_recovery_4` `revocation_hides_pulled_caches_only` |
| AC-11 | Real serve write loop + MCP save + gate held by a second process + `comemory rebuild`; 20 random-delay kills | acknowledged writes present; others `busy`; server and MCP read new content; `busy` past `pause_wait` | kill during the page copy leaves the old content | `replica_recovery_5` `rebuild_pauses_writers_and_replaces_in_place` + `killed_rebuilds_leave_a_valid_store` |
| AC-12 | `v0.43.2` against the upgraded db; the `v0.43.2` dir with the data dir read-only (`MigrationPending`) and, separately, with only `comemory.db` read-only so the first migration's write fails while the pre-migration snapshot and `store-health.json` can be written (`MigrationFailed`; the suite runs as a non-root user, as CI's runner does, since root ignores file modes); `SIGKILL` mid-migration | refusal, bytes unchanged; actionable error, no marker; chain completes; `serve` refuses to start and `sync daemon status --json` reports `healthy: false` with the state | probe never creates a db | `replica_recovery_5` `failed_upgrades_are_never_healthy` + `maintenance/tests/store_health.rs` |
| AC-13 | Coverage manifest and runner | checker exits 0; case runs the suites | unknown case exits 2 | `bash scripts/check-replication-coverage.sh`; `bash scripts/test-replication-e2e.sh --case recovery` |

## Documentation impact

- This design is the contract; `docs/README.md` links it. The five earlier
  replication designs gain a short "Changed by #256" note where this changes
  their stated behavior (seeding scope and adoption, redaction reach, the
  erased-digest barrier on local capture, rebuild progress and in-place
  replacement).
- `docs/guides/upgrading.md`: backup and restore through `comemory backup`
  replace the `cp` procedure; forward refusal; the lock order; what an
  interrupted rebuild or restore leaves; the rebuild outage.
- `docs/guides/prune-and-gc.md`: erase and what it leaves, the pending-payload
  exemption, the staged sweep and `staged_rows`, gc purge as housekeeping.
- `docs/guides/cloud-sync.md`: restores, epochs, `restore_unverified`,
  erasure and peers, revoked caches.
- `docs/configuration.md`: `[sync] pause_wait`.
- `docs/cli-reference.md` (generated), `docs/scenarios/backup.md`,
  `docs/scenarios/erase.md`, `docs/scenarios/gc.md`,
  `docs/scenarios/sync.md` (daemon status), `docs/guides/http-api.md`
  (`POST /erase`, `503 busy`, `503 restore_unverified`).
- `docs/guides/replication-e2e.md`: case `recovery`.
- `README.md`, the `AGENTS.md` command list and module map, the touched
  `src/**/README.md` files and the domain-first migration inventory rows for
  new files.

## Open Questions

None blocking. Decisions taken without a human, with the reason:

1. **The legacy database is made by the real `v0.43.2` release at test time**,
   pinned by checksum, failing rather than skipping when unavailable (Jev 1.00).
2. **A restore without an established manifest completes local-only** and
   fails closed for every sync surface, rather than refusing the restore
   (Jev 0.95 vs 0.05).
3. **Acknowledged operations a restored upstream lost are not re-published**
   (Jev 0.16 that it belongs here); Non-Goal 1.
4. **A rebuild replaces the database's content in place through the online
   backup API** instead of renaming a file over it (Jev 1.00, reassessed after
   finding that a stale connection closing last deletes the new file's WAL by
   name; the first pass had chosen retiring the old file with triggers, 0.87).
5. **Erase is one explicit command and route** (Jev 0.54 for a new verb over
   0.23 gc-only); `gc`'s purge stays housekeeping (Jev 0.96).
6. **The barrier is the erased digest, not the entity key** (Jev 0.88): new
   content under the same key is a new write.
7. **Revocation hides pulled code only on engines that loaded a policy**
   (Jev 0.93), matching how pulled documents already behave there.
8. **"Upgrade twice"** is read as opening the upgraded directory a second time
   with this binary, which must change nothing.
9. **The exchange pause and store health are interfaces** per the
   orchestrator's coordination note. They ship working against today's
   `sync.lock` and `sync daemon status`. If #257 merges while this issue's PR
   is open, the PR rewires them to the coordinator after the rebase; if it has
   not merged when this PR is ready, #257 owns the rewire, since it replaces
   both call sites.

## Review

Spec review, four rounds by an independent reviewer against the source:
round 1 found four blockers (markdown moving between replay and lock, a
dropped pending code push, un-scrubbable residue, and a rebuild clearing the
restore state) and the rename design's WAL hazard led to in-place replacement;
rounds 2–4 closed the lock-order, adoption, barrier and health gaps. Approved
2026-09-25 with no open blocker.
