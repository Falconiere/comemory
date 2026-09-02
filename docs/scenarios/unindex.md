# `comemory unindex`

Unregister a document source and delete its derived rows (`source_files`,
`documents`, chunks, FTS, `member_of_source` / `references_document`
edges). Never touches the source files on disk.

**Runnable tests:** `tests/cli__unindex.rs`, `tests/cli_scenario_documents.rs`

**HTTP:** `DELETE /api/v1/sources`, `DELETE /api/v1/sources/{target}` — covered by `tests/serve_scenario_documents.rs`

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

`<TARGET>` — source id (from `sources`) **or** the path it was registered under.

## Flags

_None besides globals._

## Scenarios

### unindex-01 By source id

- **Flags:** `--json`
- **Setup:** indexed fixtures
- **Command:** `comemory unindex <source_id> --json`
- **Expect:** `documents_removed` equals the fixture count; source files
  on disk are byte-identical; document tables empty.
- **Covered by:** `tests/cli__unindex.rs::unindex_by_source_id_removes_derived_rows_but_leaves_files_untouched`

### unindex-02 By path

- **Flags:** _(none)_
- **Command:** `comemory unindex /canonical/path`
- **Expect:** resolves the registered canonical path and removes derived rows.
- **Covered by:** `tests/cli__unindex.rs::unindex_by_path_resolves_the_registered_canonical_path`

### unindex-03 Unknown target

- **Flags:** _(none)_
- **Command:** `comemory unindex does-not-exist`
- **Expect:** not-found (non-zero).
- **Covered by:** `tests/cli__unindex.rs::unindex_unknown_target_is_not_found`
