# `comemory sources`

List registered document sources with per-status file counts. Always
reconciles (`reconcile: true`) before listing.

**Runnable tests:** `tests/cli__sources.rs`, `tests/cli_scenario_documents.rs`

**HTTP:** `GET /api/v1/sources` — covered by `tests/serve_scenario_documents.rs`

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

_None._

## Flags

_None besides globals._

## Scenarios

### sources-01 Empty

- **Flags:** `--json`
- **Command:** `comemory sources --json`
- **Expect:** `[]`. TTY prints `no sources registered`.
- **Covered by:** `tests/cli__sources.rs::sources_is_empty_before_any_registration`

### sources-02 After index

- **Flags:** `--json`
- **Setup:** `comemory index <docs> --repo docs-corpus`
- **Command:** `comemory sources --json`
- **Expect:** one row with `indexed ≥ 1`, `canonical_path`, `status`.
- **Covered by:** `tests/cli__sources.rs::sources_lists_registered_source_with_counts`
