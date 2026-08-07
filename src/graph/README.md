# graph/

**What belongs here:** the SQL-backed `edges` relation graph — typed-edge
upserts and recursive-CTE walks (`edges`), memory-body reference extraction
(`cross_link`), git co-change mining (`cochange`), per-language import
resolution (`imports`), deterministic weighted PageRank (`pagerank`), the
`index-code` post-pass that materializes mined pairs/imports and projects
PageRank onto `code_symbols.rank_score` (`materialize`), the same PageRank
projected onto `memories.rank_score` (`memory_rank`), commit co-activation
reinforcement (`coactivate`), markdown-link derivation (`doc_link`), the
search→edit lookback (`search_edit`), and the single best-effort refresh
entry point every write seam calls (`derived`).

**What does NOT belong here:** consuming the graph to rank search results.
`retrieval::graph_route` and `retrieval::code_prior` read `edges` /
`code_symbols.rank_score` to build ranking priors; this module only writes and
walks the graph, it never reranks.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `coactivate.rs` | `harvest` | Commit co-activation reward: commits touching a memory's referenced files reinforce it |
| `cochange.rs` | `CoChange` | Git co-change mining: files that change together in bounded history, weighted pairs |
| `cross_link.rs` | `Refs` | Extract `<repo>:<path>[:<symbol>]` references from a memory body |
| `derived.rs` | `refresh_derived_best_effort` | Single post-write pass refreshing both `rank_score` and the `edge_fts` index |
| `doc_link.rs` | `derive_after_document` | Deterministic `member_of_source` / `references_document` link deriver |
| `edges.rs` | `insert` | SQLite-backed edge store (replaces the v0.1 kuzu writer) |
| `imports.rs` | `extract_imports` | Per-language import extraction and conservative module-to-path resolution |
| `materialize.rs` | `materialize` | `index-code` post-pass: mined pairs + resolved imports → edges + projected PageRank |
| `memory_rank.rs` | `materialize_memory_rank` | PageRank over the derived memory graph → `memories.rank_score` |
| `pagerank.rs` | `pagerank` | Deterministic PageRank over a weighted directed graph |
| `search_edit.rs` | `memories_seen_recently` | Which memories appeared on a recent search/context page, for `auto_search_edit` provenance |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/graph.rs` (`pub mod
<name>;`) and callers import concrete paths.
