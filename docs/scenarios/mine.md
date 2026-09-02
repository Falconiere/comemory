# `comemory mine`

Distill failed → reworded search pairs (with used feedback on the
rewording) into term-expansion mappings. Report-only unless `--apply`.

**Runnable tests:** `tests/cli__mine.rs`, `tests/cli_scenario_learning.rs`

**HTTP:** `POST /api/v1/mine` — covered by `tests/serve_scenario_learning.rs`

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

_None._

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--apply` | off | Rebuild `query_expansions` from the mined set |

## Scenarios

### mine-01 Report only

- **Flags:** _(none)_
- **Setup:** a failed search plus a successful rewording with `--used` feedback
- **Command:** `comemory mine --json`
- **Expect:** `applied=false`; `mappings` non-empty; table untouched.
  TTY footer says `report only`.
- **Covered by:** `tests/cli__mine.rs::mine_reports_and_apply_rebuilds_query_expansions`

### mine-02 Apply

- **Flags:** `--apply`
- **Command:** `comemory mine --apply --json`
- **Expect:** `applied=true`; `query_expansions` row count equals mapping
  count. TTY footer says `(applied)`.
- **Covered by:** `tests/cli__mine.rs`, `tests/cli_scenario_learning.rs`
