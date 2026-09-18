# `src/domains/graph/`

**What belongs here:** the graph capability end to end — the exported model
(`code_graph`), the algorithms built on top of the `edges` relation
(memory-body reference extraction, git co-change mining, per-language import
resolution, deterministic weighted PageRank and its two projections, commit
co-activation reinforcement, markdown-link derivation, the search→edit
lookback, and the single best-effort refresh every write seam calls), the query
assembly both transports share (`query`, `nodes`), and the four command cores
`cli::` and `serve::routes::` call (`view`, `graph_nodes`, `graph_recompute`,
`edges`).

**What does NOT belong here:** SQL, rendering, or ranking. Every SQL string
lives in `store::edges`, `store::edges_retrieval`, `store::code_graph_edges`,
`store::code_graph_nodes` and `store::edge_fts`; the JSON/DOT/HTML writers and
`graph_template.html` stay in `output::graph`, which imports this folder's
model rather than owning it; and consuming the graph to rank search results
belongs to `retrieval::graph_route` and `retrieval::code_prior`.

No file here may import `cli`, `serve`, `output` or the legacy `api` tree, and
none does — `scripts/architecture-policy.json`'s `legacy_edges` allowlist is
empty once this slice lands, and `scripts/architecture-check.sh` enforces that.
`graph_nodes.rs` reads a cited memory's title from
`domains::memories::nav::title_of`, where
[#169](https://github.com/Falconiere/comemory/issues/169) moved that memory
rule out of delivery; the call travels the approved `domains::graph` →
`domains::memories` owner dependency, so no exemption is recorded for it.
Adding any import of `cli`, `serve`, `output` or `api` here fails the
architecture gate.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `coactivate.rs` | `harvest` | Commit co-activation reward: commits touching a memory's referenced files reinforce it |
| `cochange.rs` | `CoChange` | Git co-change mining: files that change together in bounded history, weighted pairs |
| `code_graph.rs` | `CodeGraph` | The exported graph model — `Node`, `Edge`, `CodeGraph` and the paginated `GraphPage`; `output::graph` renders it, this file defines it |
| `cross_link.rs` | `Refs` | Extract `<repo>:<path>[:<symbol>]` references from a memory body; URLs and bare-scheme path expressions (`file:/…`, `./…`, `../…`) are refused |
| `derived.rs` | `refresh_derived_best_effort` | Single post-write pass refreshing both `rank_score` and the `edge_fts` index |
| `doc_link.rs` | `derive_after_document` | Deterministic `member_of_source` / `references_document` link deriver |
| `edges.rs` | `run` | `comemory edges` / `GET /api/v1/edges`: the `edge_fts` self-heal and the paged triplet search |
| `edges_result.rs` | `EdgesResult` | The owned value `edges::run` returns for both delivery surfaces |
| `graph_nodes.rs` | `list` | `GET /graph/nodes`, `/nodes/{id}`, `/nodes/{id}/neighbors` and `/graph/snapshot` |
| `graph_recompute.rs` | `run` | `POST /graph/recompute`: re-project PageRank per repo in one transaction, then the memory rank |
| `imports.rs` | `extract_imports` | Per-language import extraction and conservative module-to-path resolution |
| `materialize.rs` | `materialize` | `index-code` post-pass: mined pairs + resolved imports → edges + projected PageRank; `recompute_rank(tx, repo)` is the PageRank + projection tail, shared with `graph_recompute` |
| `memory_rank.rs` | `materialize_memory_rank` | PageRank over the derived memory graph → `memories.rank_score` |
| `neighbors.rs` | `file_neighbors` | One-hop undirected `imports`/`co_changed` file neighborhood, shared by `retrieval::bundle` and `GET /api/v1/graph/nodes/{id}/neighbors` |
| `nodes.rs` | `build_graph` | The `BTreeSet` dedup of a windowed edge list into distinct `(repo, path)` pairs, and the assembly that joins node rows to edges, materializing zero-rank placeholders for endpoints the code index has never seen |
| `pagerank.rs` | `pagerank` | Deterministic PageRank over a weighted directed graph |
| `query.rs` | `build_graph_page` | The `Rel` relation vocabulary and the edge-window query behind the full and paged graph builders |
| `search_edit.rs` | `memories_seen_recently` | Which memories appeared on a recent search/context page, for `auto_search_edit` provenance |
| `view.rs` | `run` | `GET /api/v1/graph`: the full graph or one `(limit, offset)` window. `comemory graph` is deliberately not routed through it — it always wants a page, so it calls `query::build_graph_page` directly |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/domains/graph.rs` (`pub mod
<name>;`) and callers import concrete paths.
