# `comemory index-code`

Walk a git working tree, extract symbols (rust/ts/js/py/go), upsert
`code_symbols` + FTS, mine import/co-change edges, materialize PageRank.
`--extract` skips the DB and prints ingest-compatible JSONL.

**Runnable tests:** `tests/cli__index_code.rs`, `tests/cli__index_code_2.rs`,
`tests/cli_scenario_code.rs`, `tests/cli_scenario_vectors.rs`

**HTTP:** `POST /api/v1/code/index` — covered by `tests/serve_scenario_getting_started.rs`, `tests/serve_scenario_code.rs`, `tests/serve_scenario_hooks.rs`

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

_None._ `--repo` and `--path` are required flags.

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--repo` | **required** | Label stored on every symbol row. A linked `git worktree` is never a repository of its own: if `--path` is one and this label is unknown to the store, the run files under the main worktree's label instead (see [auto-reindex](../guides/auto-reindex.md#worktrees)) |
| `--path` | **required** | Working-tree root (must live inside a git repo) |
| `--extract` | off | JSONL on stdout, no DB writes, ignores the blob cursor |
| `--mode` | `incremental` | `incremental` skips unchanged blobs; `full` re-extracts every file (lossy: drops that repo's `code_vec` and per-symbol access counters) |

## Scenarios

### index-code-01 Incremental index

- **Flags:** `--repo` `--path`
- **Setup:** a real git repo with `pub fn` / `mod` sources, committed
- **Command:** `comemory index-code --repo r --path /path/to/repo`
- **Expect:** `code_symbols` ≥ 1; import + co-change edges; `rank_score`
  materialized. A second run on an unchanged tree does not grow the table.
- **Covered by:** `tests/cli__index_code.rs`, `tests/cli__index_code_2.rs::index_code_writes_symbols_and_skips_unchanged_on_rerun`

### index-code-02 Extract JSONL

- **Flags:** `--extract`
- **Command:** `comemory index-code --repo r --path /path/to/repo --extract`
- **Expect:** stdout is JSONL with `repo`, `path`, `blob_oid`, `symbol`,
  `kind`, `lang`, `line_start`, `line_end`, `snippet`, `simhash` — the
  ingest-code contract minus `embedding`.
- **Covered by:** `tests/cli__index_code_2.rs::index_code_extract_emits_ingest_compatible_jsonl`

### index-code-03 Mode full

- **Flags:** `--mode`
- **Setup:** indexed repo, then a `code_vec` row for one symbol
- **Command:** `comemory index-code --repo r --path /path/to/repo --mode full`
- **Expect:** the unchanged file is re-extracted (`files_indexed` > 0 on a
  tree that incremental would skip). BYO vectors for that repo are dropped.
- **Covered by:** `src/api/tests/index_code.rs::full_mode_re_extracts_an_unchanged_file_and_drops_its_code_vectors` (API core; the CLI only maps `--mode`)

### index-code-04 Non-git path

- **Flags:** `--path`
- **Command:** `comemory index-code --repo x --path /tmp/not-a-repo`
- **Expect:** non-zero exit (git discovery failure).
- **Covered by:** `src/api/tests/index_code.rs::run_on_a_non_git_directory_errors` (API); CLI surfaces the same error

### index-code-05 A linked worktree is not a repository

- **Flags:** `--repo` `--path`
- **Setup:** a repo `sample-repo` plus `git worktree add ../feature-42`
- **Command:** `comemory index-code --repo feature-42 --path ../feature-42`
- **Expect:** the symbols land under `sample-repo` and no `repo_marker` row is
  created for `feature-42`, so a stale git hook (or any caller passing the
  checkout's own basename) cannot add a repository per worktree. A label the
  store already knows is left alone.
- **Covered by:** `src/api/tests/index_code.rs::run_files_a_linked_worktree_under_its_main_repo_label`,
  `src/api/tests/index_code.rs::run_leaves_an_already_known_label_alone_even_in_a_worktree`,
  `src/api/tests/index_code.rs::run_keeps_a_custom_label_for_a_main_checkout`
