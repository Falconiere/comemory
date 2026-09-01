# `src/cli/graph/`

Node assembly for `comemory graph`. The parent `src/cli/graph.rs` owns the
CLI surface (`Args`, `run`, the `--format` renderers), the edge fetch, and the
two `build_code_graph` / `build_graph_page` entry points that `api::graph` and
the `serve` graph handler both reuse. This folder owns everything that turns a
`(repo, path)` pair into a graph node.

The split exists because the node query grew two columns for the console's
selected-node panel — `memories` and `blob` — and the donor file was at the
300-code-line ceiling.

| File | Responsibility |
| --- | --- |
| `nodes.rs` | `NodeRow` and its aggregate query (`fetch_nodes` for the whole graph, `fetch_nodes_for_edges` + `fetch_node_chunk` for a windowed page), plus `build_graph`, which joins node rows to edges and materializes zero-rank placeholder nodes for endpoints the code index has never seen. `EXTRA_COLUMNS` holds the blob lookup and the referencing-memory count once, so the windowed and unwindowed queries cannot drift. |
