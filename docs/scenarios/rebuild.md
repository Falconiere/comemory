# `comemory rebuild`

Replace `comemory.db`'s content from `memories/*.md`, in place through
SQLite's online backup API — one write transaction, no rename, so every
connection already open on the file (a running `serve`, an MCP session) reads
the rebuilt content on its next query. Preserves the code index, document
index, learning-loop tables and replica state by copying them from the live
database. Does **not** repopulate `memory_vec` (BYO-vector).
Emits nothing on success; `--json` is accepted with no payload.

**Runnable tests:** `tests/cli__rebuild.rs`, `tests/cli__rebuild_2.rs`,
`tests/cli__rebuild_3.rs`, `tests/cli_scenario_maintenance.rs`,
`tests/replica_recovery_5.rs`

**HTTP:** `POST /api/v1/doctor/rebuild`, `POST /api/v1/rebuild` — covered by `tests/serve_scenario_maintenance.rs`

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

_None._

## Flags

_None besides globals._

## Scenarios

### rebuild-01 From markdown

- **Flags:** _(none)_
- **Setup:** a saved memory, then drop or leave the db
- **Command:** `comemory rebuild`
- **Expect:** memories, FTS, and relation edges reconstructed; search still
  finds the keeper after a prune+rebuild.
- **Covered by:** `tests/cli__rebuild.rs::rebuild_reconstructs_memories_from_markdown`,
  `tests/cli_scenario_maintenance.rs`

### rebuild-02 Preserves code and learning

- **Flags:** `--json`
- **Setup:** ingested code symbols + feedback / eval history
- **Command:** `comemory rebuild`
- **Expect:** code tables and `eval_runs` survive; no `-wal`/`-shm` is removed
  from under an open connection. Failure leaves the original db untouched.
- **Covered by:** `tests/cli__rebuild.rs`, `tests/cli__rebuild_2.rs`

### rebuild-03 Documents

- **Flags:** _(none)_
- **Expect:** document tables survive; `source_roots` is restored from
  `sources.toml`, not from the old db.
- **Covered by:** `tests/cli__rebuild_3.rs`

### rebuild-04 Pauses the exchange and writers

- **Flags:** _(none)_
- **Setup:** a sync pass holding `sync.lock`; a `serve` and an MCP session
  saving in a loop
- **Command:** `comemory rebuild`
- **Expect:** waits for the pass to finish, then holds `sync.lock` (the
  exchange gate) and `memory-save.lock` for the whole rebuild: every save a
  server acknowledged is in the rebuilt database with its journal row, the
  rest failed `503 busy`. A gate still held after `[sync] pause_wait` fails
  `busy` (exit 75) having changed nothing. A rebuild killed at any point
  leaves a database that opens, passes `integrity_check` and holds the old or
  the new content.
- **Covered by:** `tests/replica_recovery_5.rs::rebuild_pauses_writers_and_replaces_in_place`,
  `tests/replica_recovery_5.rs::killed_rebuilds_leave_a_valid_store`
