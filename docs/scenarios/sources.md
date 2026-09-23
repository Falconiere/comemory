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

### sources-03 Withheld from sharing

A document that the secret scan refused is indexed and searchable locally like
any other — it is only kept out of replication. The listing is where that
decision is visible.

- **Flags:** `--json`
- **Setup:** a source holding a document whose text matches a redaction rule
- **Command:** `comemory sources --json`
- **Expect:** the row's `withheld` array carries one
  `[repository-relative path, rule]` pair per refused document, and is empty
  when nothing was refused. TTY prints one indented
  `withheld  <path>  (<rule>)` line beneath the source.
- **Covered by:**
  `src/domains/documents/tests/sources.rs::a_withheld_document_is_listed_with_the_rule_that_withheld_it`
