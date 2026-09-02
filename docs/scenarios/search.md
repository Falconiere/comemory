# `comemory search`

Ranked memory search: lexical FTS ladder, optional ANN, graph expansion,
rerank, diversify. `--json` prints hits plus a `query_id` for `feedback`.

**Runnable tests:** `tests/cli__search.rs`, `tests/cli__search_2.rs`,
`tests/cli__search_3.rs`, `tests/cli__search_legs.rs`,
`tests/cli_scenario_vectors.rs`

**HTTP:** `GET|POST /api/v1/memories/search` — covered by `tests/serve_scenario_getting_started.rs`, `tests/serve_scenario_memory_lifecycle.rs`, `tests/serve_scenario_vectors.rs`

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

`<QUERY>` — natural-language query (required).

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--k` / `--limit` | config `retrieval.top_k` | Page size. `0` = rest of the window |
| `--offset` | `0` | Skip this many ranked hits |
| `--repo` | unset | Exact repo filter |
| `--kind` | unset | Filter to one memory kind |
| `--vector` | unset | CSV embedding (1024-dim) for the ANN leg |
| `--vector-stdin` | off | JSON embedding on stdin |
| `--since` | unset | Created at or after (RFC3339 or `YYYY-MM-DD`) |
| `--until` | unset | Created at or before. Filters candidates only |
| `--as-of` | unset | `--until` plus supersede-penalty scoping. Conflicts with `--until` |
| `--only` | all domains | Repeatable/comma domains (`memory`, `code`, `document`) |
| `--path` | unset | Document glob (repeatable). Document-domain only |

## Scenarios

### search-01 Lexical hit

- **Flags:** _(none extra)_
- **Setup:** a saved memory whose body matches
- **Command:** `comemory search "advisory lock" --json`
- **Expect:** `hits` non-empty; `query_id` is `q-<date>-<8hex>` and a
  `retrieval_log` row exists.
- **Covered by:** `tests/cli__search.rs::search_finds_seeded_memory_lexically`

### search-02 Kind filter

- **Flags:** `--kind`
- **Setup:** a decision and a bug that both match lexically
- **Command:** `comemory search "advisory" --kind decision --json`
- **Expect:** only the decision id survives.
- **Covered by:** `tests/cli__search.rs::kind_filter_limits_hits_to_matching_kind`

### search-03 Time window

- **Flags:** `--since` `--until`
- **Setup:** two memories with different created dates
- **Command:** `comemory search QUERY --since 2026-01-01 --until 2026-06-01 --json`
- **Expect:** the window bounds `created`. Unparsable values are usage errors
  naming the flag. `--since` later than the cutoff is usage.
- **Covered by:** `tests/cli__search_2.rs::until_and_since_bound_the_created_window`

### search-04 As-of vs until

- **Flags:** `--as-of`
- **Setup:** a supersede pair
- **Command:** `comemory search QUERY --as-of 2026-04-01 --json`
- **Expect:** hides the later superseder and unwinds its penalty. Combined
  with `--until` is usage.
- **Covered by:** `tests/cli__search_2.rs::as_of_hides_the_later_superseder_and_unwinds_its_penalty`

### search-05 Pagination

- **Flags:** `--k` `--limit` `--offset`
- **Command:** `comemory search QUERY --limit 2 --offset 2 --json`
- **Expect:** `--limit` is a visible alias of `--k`. Offset beyond the window
  is empty with `has_more=false`.
- **Covered by:** `tests/cli__search.rs::search_offset_beyond_window_is_empty_with_no_more`

### search-06 Domain scope

- **Flags:** `--only` `--path`
- **Setup:** memories plus indexed documents
- **Command:** `comemory search QUERY --only document --path '*.md' --json`
- **Expect:** memory-excluding `--only` takes the document path. `--kind`
  plus a memory-excluding `--only` is usage.
- **Covered by:** `tests/cli__search_3.rs`, `tests/cli__search_legs.rs`

### search-07 Vector stdin

- **Flags:** `--vector-stdin`
- **Setup:** a 1024-dim saved vector
- **Command:**

```bash
echo '{"embedding":[...]}' | comemory search "knn dim guard" --vector-stdin --json
```

- **Expect:** hits include the vector-saved memory.
- **Covered by:** `tests/cli_scenario_vectors.rs::save_and_search_memory_vector_stdin`

### search-08 Repo filter

- **Flags:** `--repo`
- **Setup:** two memories sharing a token, saved under `alpha` and `beta`
- **Command:** `comemory search repofilter --repo beta --json`
- **Expect:** exactly one hit, and it carries `repo: beta`.
- **Covered by:** `tests/cli__search.rs::repo_filter_limits_hits_to_matching_repo`

### search-09 Vector CSV

- **Flags:** `--vector`
- **Setup:** a memory saved with a 1024-dim vector
- **Command:** `comemory search "knn dim guard" --vector 0.1,0.2,...`
- **Expect:** same ANN leg as `--vector-stdin` (both legs reported in
  `score_parts`); the CSV parser is the shared `cli::embedding_input` path.
- **Covered by:** `tests/cli__search_legs.rs::a_hybrid_search_reports_both_legs` (stdin twin),
  `tests/cli__save.rs::save_with_vector_csv_flag_writes_memory_vec_row` (CSV parser)
