# `comemory stats`

Corpus counters and `comemory.db` size. Must **not** create the database
on a fresh data dir.

**Runnable tests:** `tests/cli__stats.rs`, `tests/cli_scenario_getting_started.rs`

**HTTP:** `GET /api/v1/stats` — covered by `tests/serve_scenario_getting_started.rs`

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

_None._

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--repo` | unset | Scope `memories` / `trashed` / `code_symbols` / `documents` to one repo. `db_bytes`, `edges`, `repos`, `markdown_files` stay global |

## Scenarios

### stats-01 Empty dir

- **Flags:** `--json`
- **Command:** `comemory stats --json`
- **Expect:** zeros; `schema_version` is `"unknown"`; `comemory.db` is not
  created.
- **Covered by:** `tests/cli__stats.rs::stats_on_an_empty_data_dir_reports_unknown_and_creates_no_database`

### stats-02 Real corpus

- **Flags:** `--json`
- **Setup:** three saved memories
- **Command:** `comemory stats --json`
- **Expect:** `memories` matches the live count; `markdown_files` matches
  the directory listing; `db_bytes > 0`.
- **Covered by:** `tests/cli__stats.rs::stats_counts_a_real_corpus_and_agrees_with_the_filesystem`

### stats-03 Repo scope

- **Flags:** `--repo`
- **Command:** `comemory stats --repo comemory --json`
- **Expect:** memory counter is the subset for that repo.
- **Covered by:** `tests/cli__stats.rs::repo_scopes_the_memory_counter`

### stats-04 After index-code

- **Flags:** `--json`
- **Setup:** getting-started journey
- **Command:** `comemory stats --json`
- **Expect:** `memories ≥ 1` and `code_symbols ≥ 1`.
- **Covered by:** `tests/cli_scenario_getting_started.rs`
