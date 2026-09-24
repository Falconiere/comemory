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

### sources-03 Whether the source is shared at all

A source is only replicated when its `--repo` label resolves to a repository
the workspace policy approves AND that repository has an indexed root on this
machine. All four ways that can fail are reported by name, because none of them
is visible in the file counts and each has a different fix.

- **Flags:** `--json`
- **Command:** `comemory sources --json`
- **Expect:** `shared_as` carries the canonical `owner/name` when the source is
  shared, else `unshared_reason` carries one of `no repository label`,
  `no sync policy has been loaded`, ``repository `<label>` is not approved`` or
  ``repository `<label>` has no indexed root on this machine``. TTY prints one
  indented `shared as <repo>` or `not shared: <reason>` line.
- **Covered by:**
  `src/domains/documents/tests/sources.rs::an_approved_and_rooted_source_reports_the_name_it_shares_under`
  and its four sibling `a_source_says_when_*` / `a_source_with_no_label_says_so` cases

### sources-04 Withheld from sharing

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
