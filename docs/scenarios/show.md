# `comemory show`

One memory in full: body, frontmatter fields, activation, and code refs
(with freshness).

**Runnable tests:** `tests/cli__show.rs`, `tests/cli_scenario_getting_started.rs`

**HTTP:** `GET /api/v1/memories/{id}` — covered by `tests/serve_scenario_getting_started.rs`, `tests/serve_scenario_memory_lifecycle.rs`

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

`<ID>` — 8-hex memory id (required).

## Flags

_None besides globals._

## Scenarios

### show-01 Full body and metadata

- **Flags:** `--json`
- **Setup:** a memory saved with tags, `--quality 4`, and a backtick
  `repo:path:symbol` body reference
- **Command:** `comemory show <id> --json`
- **Expect:** `body` round-trips verbatim; `quality` is 4; `tags` match;
  `code_refs` has the implied file/symbol ref.
- **Covered by:** `tests/cli__show.rs::show_returns_full_body_quality_tags_and_one_code_ref`

### show-02 After getting-started save

- **Flags:** `--json`
- **Setup:** the getting-started journey
- **Command:** `comemory show <id> --json`
- **Expect:** body equals the saved string.
- **Covered by:** `tests/cli_scenario_getting_started.rs`
