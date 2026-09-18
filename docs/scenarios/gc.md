# `comemory gc`

Hard-delete `memories/.trash/` entries older than the trash retention
window, purge their mirror rows, and evict `retrieval_log` /
`feedback_events` past learning retention. Must **not** create the
database on a fresh data dir.

Captured candidate observations ([`judge`](judge.md)) age out on the same
learning-retention window, with one rule of their own: an observation carrying
a reviewed judgment is retained however old it is, because evicting it would
leave a verdict with no passage to verify it against. Purging a trashed memory
also redacts its captured passages — the candidate row, its pool position and
its reference stay, so the recorded pool keeps its shape, but the body is
blanked and the candidate is marked unresolvable.

**Runnable tests:** `tests/cli__gc.rs`, `tests/cli_scenario_maintenance.rs`,
`tests/cli__judge.rs`

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
- **Expect:** `removed=0`, `log_rows=0`, `event_rows=0`, `observation_rows=0`;
  no `comemory.db`.
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

### gc-04 Candidate observation retention and purge redaction

- **Flags:** `--json`
- **Setup:** two captured observations, one of them judged, both aged past the
  learning-retention window; separately, a captured memory candidate whose
  memory is then deleted and purged
- **Command:** `comemory gc --json`
- **Expect:** `observation_rows=1` — the unjudged observation and its candidate
  rows are evicted while the judged one and its candidates are retained. The
  purged memory's candidate row survives with its pool position and reference
  intact, an empty `text` and `unresolved = 1`; another domain's candidate in
  the same observation is untouched, and judging the redacted candidate is
  refused.
- **Covered by:** `tests/cli__judge.rs::gc_evicts_unjudged_observations_and_keeps_judged_ones`,
  `tests/cli__judge.rs::purging_a_memory_redacts_its_captured_text_and_keeps_the_pool_shape`
