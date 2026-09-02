# `comemory graph`

Export the file-level code-connection graph (imports + co-change) as
JSON, Graphviz DOT, or interactive HTML. Pure read over `comemory.db`.
Global `--json` forces JSON regardless of `--format`.

**Runnable tests:** `tests/cli__graph.rs`, `tests/cli__graph_2.rs`,
`tests/cli__graph_3.rs`, `tests/cli_scenario_code.rs`

**HTTP:** `GET /api/v1/graph` — covered by `tests/serve_scenario_code.rs`

Global flags `--json` and `--data-dir` apply. See [globals.md](globals.md).

## Positionals

_None._

## Flags

| Flag | Default | Effect |
| --- | --- | --- |
| `--repo` | unset | Restrict to one index label |
| `--format` | `json` | `json` \| `dot` \| `html` |
| `--rel` | `all` | `all` \| `imports` \| `co-changed` |
| `--min-weight` | `1` | Drop `co_changed` edges below this floor (≥ 1). Does not affect imports |
| `--limit` | `50` | Window over edges. `0` = full graph |
| `--offset` | `0` | Skip this many edges |

## Scenarios

### graph-01 Indexed edges

- **Flags:** `--repo` `--json`
- **Setup:** two-file rust repo with `mod b;` and two joint commits
- **Command:** `comemory graph --repo r --json`
- **Expect:** nodes `file:r:a.rs` / `file:r:b.rs`; `imports` and
  `co_changed` (weight 2) edges.
- **Covered by:** `tests/cli__graph.rs::graph_emits_indexed_edges_and_gates_co_changed_weight`

### graph-02 Min-weight

- **Flags:** `--min-weight`
- **Command:** `comemory graph --repo r --min-weight 3 --json`
- **Expect:** weight-2 `co_changed` dropped; `imports` survives. `--min-weight 0` is usage.
- **Covered by:** `tests/cli__graph.rs`

### graph-03 Format

- **Flags:** `--format` `--json`
- **Command:** `comemory graph --format dot` / `--format html` / `--format dot --json`
- **Expect:** DOT contains `digraph` or `->`; HTML is a page; global `--json`
  wins over `--format dot`.
- **Covered by:** `tests/cli__graph.rs`, `tests/cli_scenario_code.rs`

### graph-04 Rel and paging

- **Flags:** `--rel` `--limit` `--offset`
- **Command:** `comemory graph --rel imports --limit 1 --json`
- **Expect:** only import edges; window applies to every format.
- **Covered by:** `tests/cli__graph.rs::graph_rel_filters_edge_kinds` (rel), `tests/cli__graph_2.rs` (paging)
