# `comemory delete`

Soft-delete one memory: move the markdown into `memories/.trash/` (source
of truth), stamp `deleted_at` on the mirror row, drop FTS/edges, refresh
derived rank.

**Runnable tests:** `tests/cli__delete.rs`, `tests/cli_scenario_memory_lifecycle.rs`

**HTTP:** `DELETE /api/v1/memories/{id}` — covered by `tests/serve_scenario_memory_lifecycle.rs`

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

`<ID>` — 8-hex memory id (required).

## Flags

_None besides globals._

## Scenarios

### delete-01 Soft-delete a live memory

- **Flags:** _(none)_
- **Setup:** a saved memory
- **Command:** `comemory delete <id>`
- **Expect:** TTY `deleted <id>`; `deleted_at` set; FTS row gone; edges
  gone; `list` / `search` no longer show it; a file remains under
  `memories/.trash/`.
- **Covered by:** `tests/cli__delete.rs::delete_stamps_deleted_at_in_sqlite`

### delete-02 Missing id

- **Flags:** _(none)_
- **Setup:** fresh data dir (no memories yet)
- **Command:** `comemory delete deadbeef0000`
- **Expect:** failure; stderr `memory not found`; **not** a raw ENOENT.
  `ensure_dirs` runs before the store open.
- **Covered by:** `tests/cli__delete.rs::delete_missing_id_fails_without_enoent`

### delete-03 Rank redistribution

- **Flags:** _(none)_
- **Setup:** a hub memory superseded by two survivors
- **Command:** `comemory delete <hub>`
- **Expect:** survivors' `rank_score` changes (mass redistributes).
- **Covered by:** `tests/cli__delete.rs::delete_redistributes_memory_rank_to_survivors`
