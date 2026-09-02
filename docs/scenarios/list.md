# `comemory list`

Page live memories from the SQLite mirror. Filters and the window are
pushed into SQL so cost scales with the page, not the corpus.

**Runnable tests:** `tests/cli__list.rs`, `tests/cli_scenario_memory_lifecycle.rs`

**HTTP:** `GET /api/v1/memories` — covered by `tests/serve_scenario_memory_lifecycle.rs`

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

_None._

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--repo` | unset | Exact repo match |
| `--kind` | unset | Case-insensitive kind filter |
| `--tag` | unset | Memories carrying this exact tag |
| `--min-quality` | unset | Quality ≥ this (1..=5) |
| `--query` | unset | Case-insensitive substring of the body |
| `--sort` | `created` | `created` (newest first) \| `quality` \| `accessed` |
| `--limit` | `50` | Page size. `0` = all remaining |
| `--offset` | `0` | Skip this many rows |

## Scenarios

### list-01 Page envelope

- **Flags:** `--json`
- **Setup:** two saved memories
- **Command:** `comemory list --json`
- **Expect:** object `{items, limit, offset, total, has_more}` (not a bare
  array). Each row carries `id`, `kind`, `repo`, `slug`, `title`, `tags`,
  `quality`, `created`, `access_count`.
- **Covered by:** `tests/cli__list.rs::list_json_is_page_envelope_not_bare_array`

### list-02 Repo and kind

- **Flags:** `--repo` `--kind`
- **Setup:** six memories across two repos and two kinds
- **Command:** `comemory list --repo alpha --kind decision --json`
- **Expect:** only alpha+decision rows. TTY footer `showing 1–N of N`.
- **Covered by:** `tests/cli__list.rs::list_json_repo_and_kind_filters_combine`

### list-03 Sort

- **Flags:** `--sort`
- **Command:** `comemory list --sort quality --json`
- **Expect:** descending quality. `--sort accessed` puts the
  most-recently-searched first. Default is newest-created.
- **Covered by:** `tests/cli__list.rs::list_json_sort_quality_orders_descending`

### list-04 Pagination

- **Flags:** `--limit` `--offset`
- **Command:** `comemory list --limit 2 --offset 4 --json`
- **Expect:** last page `has_more=false`. `--limit 0` returns all.
- **Covered by:** `tests/cli__list.rs::list_json_limit_and_offset_page_correctly`

### list-05 Tag, min-quality, query

- **Flags:** `--tag` `--min-quality` `--query`
- **Setup:** a quality-5 note tagged `zebra` whose body contains `zebra`,
  plus a quality-1 untagged note
- **Command:** `comemory list --tag zebra --min-quality 5 --query zebra --json`
- **Expect:** the tagged high-quality row is present; the low-quality
  untagged row is not.
- **Covered by:** `tests/cli__list.rs::list_json_tag_min_quality_and_query_filters`
