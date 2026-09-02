# `comemory search-code`

Ranked search over `code_symbols` (BM25 + optional ANN, graph priors).
Working-set affinity applies only when the process CWD is the indexed
repo **and** `--repo` matches the label used at `index-code` time.

**Runnable tests:** `tests/cli__search_code.rs`, `tests/cli__search_code_2.rs`,
`tests/cli__lazy_reindex.rs`, `tests/cli_scenario_code.rs`,
`tests/cli_scenario_vectors.rs`

**HTTP:** `GET|POST /api/v1/code/search` — covered by `tests/serve_scenario_getting_started.rs`, `tests/serve_scenario_code.rs`, `tests/serve_scenario_vectors.rs`

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

`<QUERY>` — natural-language or identifier query (required). CamelCase /
snake_case tokens split automatically.

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--k` / `--limit` | config `retrieval.top_k` | Page size |
| `--offset` | `0` | Skip this many ranked hits |
| `--repo` | unset | Repo label as passed to `index-code --repo` |
| `--lang` | unset | Language filter (`rust`/`rs`, `python`/`py`, …) |
| `--vector` | unset | CSV embedding (768-dim) |
| `--vector-stdin` | off | JSON embedding on stdin |

## Scenarios

### search-code-01 Lexical with repo and lang

- **Flags:** `--repo` `--lang`
- **Setup:** a git repo indexed with rust and python symbols sharing tokens
- **Command:** `comemory search-code "alpha router" --repo r --lang rust --json`
- **Expect:** hits are rust-only; `query_id` present; `symbol_id` is a
  positive i64; `score_parts` carries relevance/rank/activation/affinity/feedback.
- **Covered by:** `tests/cli__search_code.rs::lang_filter_narrows_hits_to_one_language`

### search-code-02 Pagination

- **Flags:** `--k` `--limit` `--offset`
- **Setup:** many symbols
- **Command:** `comemory search-code QUERY --limit 2 --offset 2 --json`
- **Expect:** order is stable across offsets; `--limit` aliases `--k`;
  offset beyond the window is empty with `has_more=false`.
- **Covered by:** `tests/cli__search_code_2.rs`

### search-code-03 Vector stdin

- **Flags:** `--vector-stdin`
- **Setup:** symbols ingested with 768-dim embeddings
- **Command:**

```bash
echo '{"embedding":[...768 floats...]}' | \
  comemory search-code "parse frontmatter" --vector-stdin --json
```

- **Expect:** hits the ingested symbol.
- **Covered by:** `tests/cli_scenario_vectors.rs::extract_embed_ingest_search_code_vector`

### search-code-04 Feedback used-code

- **Flags:** _(output consumed by feedback)_
- **Setup:** indexed repo, a `query_id` from this command
- **Command:** `comemory feedback <query_id> --used-code <symbol_id>`
- **Expect:** exit 0. Round-trip is the e2e smoke and the code journey.
- **Covered by:** `tests/cli_scenario_code.rs`, `scripts/e2e.sh`

### search-code-05 Lazy reindex

- **Flags:** _(none extra)_
- **Setup:** `indexing.auto_reindex=lazy`, repo HEAD moved since last index
- **Command:** `comemory search-code QUERY --repo r`
- **Expect:** search returns immediately on the current index; a detached
  `index-code` may spawn. A bad `--lang` fails **before** the trigger.
- **Covered by:** `tests/cli__lazy_reindex.rs`

### search-code-06 Vector CSV

- **Flags:** `--vector`
- **Setup:** an ingested 768-dim code vector
- **Command:** `comemory search-code "parse frontmatter" --vector 0.1,0.2,...`
- **Expect:** same ANN leg as `--vector-stdin`; the CSV parser is the shared
  `cli::embedding_input` path every `--vector` flag goes through.
- **Covered by:** `tests/cli_scenario_vectors.rs::extract_embed_ingest_search_code_vector` (stdin twin),
  `tests/cli__save.rs::save_with_vector_csv_flag_writes_memory_vec_row` (CSV parser)
