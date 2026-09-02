# `comemory index`

Register one or more files or directories as document sources and run
their synchronous initial reconcile (extract + chunk + FTS).

**Runnable tests:** `tests/cli__index.rs`, `tests/cli_scenario_documents.rs`

**HTTP:** `POST /api/v1/sources` — covered by `tests/serve_scenario_documents.rs`

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

`<PATH>...` — one or more files or directories (required).

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--repo` | git worktree basename, if any | Label attached to every document under each source |
| `--strict` | off | Exit 65 after the run if any per-file error occurred. Partial success still attempts every file |

## Scenarios

### index-01 Register fixtures

- **Flags:** `--json`
- **Setup:** `tests/common/fixtures/docs/` copied into a temp dir
- **Command:** `comemory index /tmp/docs --json`
- **Expect:** one source; `indexed` equals the four fixtures (md/txt/html/csv).
- **Covered by:** `tests/cli__index.rs::index_registers_and_indexes_real_fixtures`

### index-02 Re-register is idempotent

- **Flags:** _(none extra)_
- **Command:** `comemory index /tmp/docs` twice
- **Expect:** second run `indexed=0`, `unchanged=4`; still one `source_roots` row.
- **Covered by:** `tests/cli__index.rs::reregistering_same_path_updates_not_duplicates`

### index-03 Repo label

- **Flags:** `--repo`
- **Command:** `comemory index /tmp/docs --repo docs-corpus`
- **Expect:** `sources --json` shows `repo=docs-corpus`.
- **Covered by:** `tests/cli__sources.rs::sources_lists_registered_source_with_counts`

### index-04 Strict

- **Flags:** `--strict`
- **Setup:** a source that produces per-file errors (or none)
- **Command:** `comemory index /tmp/docs --strict --json`
- **Expect:** exit 65 only when errors occurred; overlap with an existing
  source is rejected with "overlaps" on stderr.
- **Covered by:** `tests/cli__index.rs`

### index-05 Document journey

- **Flags:** _(none extra)_
- **Command:** `index` → `sources` → `find --domain document` → `unindex`
- **Covered by:** `tests/cli_scenario_documents.rs`
