# `comemory context`

Headline lookup: matching code symbol(s) plus memories for a key.
`--json` is a bundle (`query`, `memories`, `symbol`, neighbors).

**Runnable tests:** `tests/cli__context.rs`, `tests/cli__context_2.rs`,
`tests/cli_scenario_getting_started.rs`

**HTTP:** `GET|POST /api/v1/context` — covered by `tests/serve_scenario_getting_started.rs`

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

`<QUERY>` — key (symbol name, file fragment, or phrase).

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--k` / `--limit` | config `retrieval.top_k` | How many memories in the bundle |
| `--offset` | `0` | Skip this many memory hits |
| `--repo` | unset | Scope both legs |
| `--vector` | unset | CSV embedding (1024-dim) |
| `--vector-stdin` | off | JSON embedding on stdin |
| `--since` | unset | Memory created-at lower bound |
| `--until` | unset | Memory created-at upper bound |
| `--as-of` | unset | As-of supersede semantics. Conflicts with `--until` |

## Scenarios

### context-01 Lexical bundle

- **Flags:** `--json`
- **Setup:** a saved memory matching the key
- **Command:** `comemory context "advisory lock" --json`
- **Expect:** `query` echoes the key; `memories` non-empty. Each memory row's
  `score` is the pipeline's `final_score` for that memory — the same number
  `comemory search --json` reports — in ranked (non-increasing) order, never a
  `0.0` placeholder.
- **Covered by:** `tests/cli__context.rs::context_returns_bundle_for_seeded_memory`,
  `tests/cli__context.rs::context_memory_rows_carry_the_pipeline_score`,
  `tests/api__context.rs::run_carries_the_search_pipeline_score_onto_each_memory_row`

### context-02 With code

- **Flags:** `--repo`
- **Setup:** indexed repo + a memory about the same topic
- **Command:** `comemory context frontmatter --repo demo --json`
- **Expect:** bundle carries memories (and a symbol when one matches).
- **Covered by:** `tests/cli_scenario_getting_started.rs`

### context-03 Vector stdin

- **Flags:** `--vector-stdin`
- **Setup:** a memory with a stored 1024-dim vector
- **Command:** `echo '{"embedding":[...]}' | comemory context "vector path" --vector-stdin --json`
- **Expect:** bundle shape valid (ANN branch).
- **Covered by:** `tests/cli__context.rs::context_vector_path_accepts_stdin_vector`

### context-04 Time window

- **Flags:** `--since` `--until` `--as-of` `--k` `--limit` `--offset`
- **Expect:** same grammar as `search`.
- **Covered by:** `tests/cli__context_2.rs`

### context-06 A `file:` URL is not a code reference

- **Flags:** `--repo` `--json`
- **Setup:** a memory whose body records a database location as a bare-scheme
  URL (`file:/tmp/check.db`, `sqlite:./data.db`, `file:../../local.db`)
- **Command:** `comemory context gate --repo comemory.io --json`
- **Expect:** the memory is surfaced; `code_refs` and `relations` are empty —
  `cross_link` refuses a captured path that starts with `/`, `./` or `../`,
  so no `references_file` edge to a pseudo-repo named after the scheme exists
  (issue #153). Migration `0018` removes the ones an older binary wrote.
- **Covered by:** `tests/cli__context.rs::context_does_not_cite_a_file_url_as_a_code_reference`,
  `src/domains/graph/tests/cross_link.rs::scheme_prefixed_path_expressions_are_not_refs`,
  `src/store/tests/migrate.rs::the_scheme_path_migration_drops_junk_refs_and_keeps_real_ones`

### context-05 Vector CSV

- **Flags:** `--vector`
- **Setup:** a memory saved with a 1024-dim vector
- **Command:** `comemory context "frontmatter" --vector 0.1,0.2,...`
- **Expect:** same ANN leg as `--vector-stdin`; the CSV parser is the shared
  `utilities::embedding_input` path every `--vector` flag goes through.
- **Covered by:** `tests/cli__context.rs::context_vector_path_accepts_stdin_vector` (stdin twin),
  `tests/cli__save.rs::save_with_vector_csv_flag_writes_memory_vec_row` (CSV parser)
