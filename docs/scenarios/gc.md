# `comemory gc`

Hard-delete `memories/.trash/` entries older than the trash retention
window, purge their mirror rows, and evict `retrieval_log` /
`feedback_events` past learning retention. Must **not** create the
database on a fresh data dir.

**Runnable tests:** `tests/cli__gc.rs`, `tests/cli_scenario_maintenance.rs`

**HTTP:** `POST /api/v1/gc` — covered by `tests/serve_scenario_maintenance.rs`

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

_None._

## Flags

_None besides globals._

## Scenarios

### gc-01 Fresh dir

- **Flags:** `--json`
- **Command:** `comemory gc --json`
- **Expect:** `removed=0`, `log_rows=0`, `event_rows=0`; no `comemory.db`.
- **Covered by:** `tests/cli__gc.rs::gc_on_fresh_dir_does_not_create_db`

### gc-02 Aged trash

- **Flags:** `--json`
- **Setup:** `delete` a memory, backdate the trash file past 30 days
- **Command:** `comemory gc --json`
- **Expect:** `removed=1`; `bytes_freed` equals the file size; a `gc_runs` row.
- **Covered by:** `tests/cli__gc.rs::gc_reports_bytes_freed_and_writes_a_gc_runs_row`

### gc-03 Telemetry retention

- **Flags:** _(none)_
- **Setup:** old `retrieval_log` / `feedback_events` rows
- **Command:** `COMEMORY_LEARNING_RETENTION_DAYS=7 comemory gc --json`
- **Expect:** old rows evicted; counters and expansions kept. Env override
  of 200 days keeps 100-day rows.
- **Covered by:** `tests/cli__gc.rs`
