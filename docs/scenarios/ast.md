# `comemory ast`

Run an ast-grep pattern against a single source file. No database.
Supported langs: rust, typescript, javascript, python, go (and aliases
`rs`/`ts`/`js`/`py`).

**Runnable tests:** `tests/cli__ast.rs`, `tests/cli_scenario_code.rs`

**HTTP:** `POST /api/v1/code/ast` — covered by `tests/serve_scenario_code.rs`

Global flags `--json` and `--data-dir` apply (`--data-dir` is accepted
but unused). See [globals.md](globals.md).

## Positionals

`<PATTERN>` — ast-grep pattern (`$VAR`, `$$$ARGS`, …).

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--lang` | **required** | `rs`/`rust`, `ts`/`tsx`/`typescript`, `js`/`jsx`/`javascript`, `py`/`python`, `go` |
| `--file` | **required** | Source file to search |
| `--limit` | `50` | Page size over matches. `0` = all |
| `--offset` | `0` | Skip this many matches |

## Scenarios

### ast-01 Rust functions

- **Flags:** `--lang` `--file` `--json`
- **Setup:** a `.rs` file with `fn alpha() { … }`
- **Command:** `comemory ast 'fn $NAME($$$) { $$$ }' --lang rs --file a.rs --json`
- **Expect:** `Page.items` contains a match whose `text` includes `alpha`.
- **Covered by:** `tests/cli_scenario_code.rs`

### ast-02 Pagination

- **Flags:** `--limit` `--offset`
- **Setup:** N `tokio::spawn` sites
- **Command:** `comemory ast 'tokio::spawn($$$)' --lang rs --file f.rs --limit 2 --offset 1 --json`
- **Expect:** two items; `has_more` true when more remain. `--limit 0` returns all.
- **Covered by:** `tests/cli__ast.rs`

### ast-03 Unsupported lang

- **Flags:** `--lang`
- **Command:** `comemory ast pattern --lang ruby --file x.rs`
- **Expect:** failure; stderr lists `supported:`.
- **Covered by:** `tests/cli__ast.rs::ast_rejects_unsupported_lang`
