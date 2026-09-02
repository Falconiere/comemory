# `comemory edges`

Lexical search over the relation graph (`edge_fts`). Hits print as
`src —rel→ dst`. An empty index self-heals on first use.

**Runnable tests:** `tests/cli__edges.rs`, `tests/cli_scenario_getting_started.rs`

**HTTP:** `GET /api/v1/edges` — covered by `tests/serve_scenario_getting_started.rs`

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

`<QUERY>` — words to match against rendered triplets.

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--k` / `--limit` | config `retrieval.top_k` | Page size |
| `--offset` | `0` | Skip this many hits |

## Scenarios

### edges-01 Supersede triplet

- **Flags:** `--json`
- **Setup:** `save --supersedes`
- **Command:** `comemory edges "supersedes queue design" --json`
- **Expect:** one triplet `(supersedes, new, old)`. TTY renders kind + slug.
- **Covered by:** `tests/cli__edges.rs::a_supersede_edge_is_findable_and_leaves_on_delete`

### edges-02 Pagination

- **Flags:** `--k` `--limit` `--offset`
- **Command:** `comemory edges QUERY --limit 5 --offset 0 --json`
- **Expect:** page envelope over triplets.
- **Covered by:** `tests/cli__edges.rs`

### edges-03 Getting-started

- **Flags:** _(none extra)_
- **Command:** `comemory edges frontmatter`
- **Expect:** exit 0 (empty graph is fine).
- **Covered by:** `tests/cli_scenario_getting_started.rs`
