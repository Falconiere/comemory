# `src/cli/graph/`

Node assembly for `comemory graph`. The parent `src/cli/graph.rs` owns the
CLI surface (`Args`, `run`, the `--format` renderers), the edge fetch, and the
two `build_code_graph` / `build_graph_page` entry points that `api::graph` and
the `serve` graph handler both reuse. This folder owns the pure `(repo, path)`
dedup over a windowed edge set and the `build_graph` assembly; the SQL and its
row mapping — `NodeRow`, `fetch_nodes`/`fetch_node`/`fetch_nodes_for_pairs`,
`cites_file_predicate` — moved to `store::code_graph_nodes` (spec
`docs/toolu/specs/2026-09-07-store-layer-chokepoint-design.md`).

The split exists because the node query grew two columns for the console's
selected-node panel — `memories` and `blob` — and the donor file was at the
300-code-line ceiling.

| File | Responsibility |
| --- | --- |
| `nodes.rs` | `fetch_nodes_for_edges` — the `BTreeSet` dedup of a windowed edge list into distinct `(repo, path)` pairs, delegating the batched fetch to `store::code_graph_nodes::fetch_nodes_for_pairs` — plus `build_graph`, which joins node rows to edges and materializes zero-rank placeholder nodes for endpoints the code index has never seen |
