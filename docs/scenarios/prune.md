# `comemory prune`

Detect (and optionally soft-delete) stale / low-value memories, orphan
edges, and stale code files. Default is a dry run. `--apply` always acts
on the **full** candidate set — `--limit` / `--offset` window display only.

**Runnable tests:** `tests/cli__prune.rs`, `tests/cli__prune_2.rs`,
`tests/cli_scenario_maintenance.rs`

**HTTP:** `GET|POST /api/v1/prune`, `GET /api/v1/prune/candidates` — covered by `tests/serve_scenario_maintenance.rs`

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

_None._

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--apply` | off | Soft-delete low-value memories and clean orphan/stale rows |
| `--ids` | unset | Restrict `--apply` to these 8-hex ids (comma-separated). Non-candidates are ignored |
| `--limit` | `50` | Window over the dry-run lists (both of them) |
| `--offset` | `0` | Skip this many dry-run rows |

## Scenarios

### prune-01 Dry run on a clean db

- **Flags:** `--json`
- **Command:** `comemory prune --json`
- **Expect:** zero `orphan_edges`; empty `stale_code_files` / `low_value_memories` pages.
- **Covered by:** `tests/cli__prune.rs::prune_dry_run_on_clean_db_emits_zero_counts`

### prune-02 Dry run reports without deleting

- **Flags:** _(none)_
- **Setup:** a memory doctored below the quality/activation/feedback floors
- **Command:** `comemory prune --json`
- **Expect:** that id is in `low_value_memories.items`; the markdown is still live.
- **Covered by:** `tests/cli__prune.rs::prune_dry_run_reports_low_value_memory_without_deleting`

### prune-03 Apply

- **Flags:** `--apply`
- **Command:** `comemory prune --apply --json`
- **Expect:** the low-value memory is soft-deleted (same path as `delete`).
- **Covered by:** `tests/cli__prune.rs::prune_apply_soft_deletes_low_value_memory`

### prune-04 Apply ignores the display window

- **Flags:** `--apply` `--limit` `--offset`
- **Setup:** five low-value memories, `--limit 2`
- **Command:** `comemory prune --apply --limit 2`
- **Expect:** all five are deleted, not just the two shown.
- **Covered by:** `tests/cli__prune_2.rs`

### prune-05 Apply a subset of ids

- **Flags:** `--apply` `--ids`
- **Setup:** two low-value memories
- **Command:** `comemory prune --apply --ids <only-the-first>`
- **Expect:** only that id is soft-deleted; the other remains live.
- **Covered by:** `tests/cli__prune_2.rs::prune_apply_ids_restricts_the_delete_set`,
  `src/api/tests/prune.rs::run_apply_with_ids_touches_only_the_listed_candidate`
