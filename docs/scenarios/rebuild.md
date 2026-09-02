# `comemory rebuild`

Atomically replace `comemory.db` from `memories/*.md`. Preserves the code
index, document index, and learning-loop tables by copying them from a
pre-rebuild snapshot. Does **not** repopulate `memory_vec` (BYO-vector).
Emits nothing on success; `--json` is accepted with no payload.

**Runnable tests:** `tests/cli__rebuild.rs`, `tests/cli__rebuild_2.rs`,
`tests/cli__rebuild_3.rs`, `tests/cli_scenario_maintenance.rs`

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
- **Expect:** code tables and `eval_runs` survive. WAL/SHM sidecars cleaned.
  Failure leaves the original db untouched.
- **Covered by:** `tests/cli__rebuild.rs`, `tests/cli__rebuild_2.rs`

### rebuild-03 Documents

- **Flags:** _(none)_
- **Expect:** document tables survive; `source_roots` is restored from
  `sources.toml`, not from the old db.
- **Covered by:** `tests/cli__rebuild_3.rs`
