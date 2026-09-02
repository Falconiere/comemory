# `comemory find`

One ranked list over memories, code, and documents (weighted RRF). Each
hit carries a `domain` tag. A memory-only run must order identically to
`search`.

**Runnable tests:** `tests/cli__find.rs`, `tests/cli_scenario_documents.rs`

**HTTP:** `GET|POST /api/v1/find` — covered by `tests/serve_scenario_documents.rs`

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

`<QUERY>` — natural-language query (required).

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--k` / `--limit` | config `retrieval.top_k` | Page size |
| `--offset` | `0` | Skip this many fused hits |
| `--domain` | `all` | `all` \| `memory` \| `code` \| `document` |
| `--repo` | unset | Narrows memory and code legs |
| `--kind` | unset | Narrows the memory leg only |
| `--lang` | unset | Narrows the code leg only |
| `--path` | unset | Document glob (repeatable). Document leg only |
| `--vector` | unset | CSV embedding |
| `--vector-stdin` | off | JSON embedding on stdin |
| `--since` | unset | Memory created-at lower bound |
| `--until` | unset | Memory created-at upper bound |
| `--as-of` | unset | As-of supersede semantics. Conflicts with `--until` |

## Scenarios

### find-01 Both domains

- **Flags:** `--json`
- **Setup:** a memory and a code symbol both matching "frontmatter"
- **Command:** `comemory find frontmatter --json`
- **Expect:** `hits` include `domain=memory` and `domain=code`, ordered by
  descending score. `query_id` is accepted by `feedback`.
- **Covered by:** `tests/cli__find.rs::find_returns_both_domains_in_one_ranking_ordered_by_score`

### find-02 Memory-only matches search

- **Flags:** `--domain`
- **Command:** `comemory find frontmatter --domain memory --k 5 --json`
- **Expect:** ids equal `comemory search frontmatter --k 5 --json`.
- **Covered by:** `tests/cli__find.rs::a_memory_only_find_orders_identically_to_search`

### find-03 Lang narrows code

- **Flags:** `--lang` `--domain`
- **Setup:** the same symbol in `.rs` and `.py`
- **Command:** `comemory find parse_frontmatter --domain code --lang rust --json`
- **Expect:** only `.rs` hits.
- **Covered by:** `tests/cli__find.rs::lang_actually_narrows_the_code_leg`

### find-04 Document domain

- **Flags:** `--domain`
- **Setup:** indexed `guide.md`
- **Command:** `comemory find Homebrew --domain document --json`
- **Expect:** at least one `domain=document` hit; after `unindex`, none.
- **Covered by:** `tests/cli_scenario_documents.rs`

### find-05 Unknown domain

- **Flags:** `--domain`
- **Command:** `comemory find anything --domain sideways`
- **Expect:** usage error naming `sideways`.
- **Covered by:** `tests/cli__find.rs::an_unknown_domain_is_a_usage_error_naming_the_offender`

### find-06 Time and vector flags

- **Flags:** `--since` `--until` `--as-of` `--vector` `--vector-stdin` `--path` `--repo` `--kind` `--k` `--limit` `--offset`
- **Expect:** same grammar as `search` / `search-code` for the flags they
  share. `--as-of` conflicts with `--until`.
- **Covered by:** `tests/cli__find.rs` (domain/kind/lang/paging),
  `tests/cli__search_2.rs` (time grammar is shared)
