# retrieval/

**What belongs here:** the hybrid ranking pipeline end to end — candidate
routing (`router`, `code_route`, `doc_route`), fusion (`fuse`), reranking by
bounded deterministic priors (`rerank`, `code_rerank`, `code_prior`, `score`),
diversification (`diversify`), graph expansion (`graph_route`), the
`--since`/`--until`/`--as-of` scope shared by every leg (`scope`), the
`comemory context` bundle shape (`bundle`), pinned code-reference freshness
(`code_ref_collect`/`code_ref_fetch`/`code_ref_status`), and the shared
code-search entry point (`code_search`) used by both `search-code` and
`serve`.

**What does NOT belong here:** raw SQL table/DDL access beyond what a
candidate leg needs to build its ranked list. FTS5, `vec0`, and row CRUD
primitives live in `store/`; `retrieval/` calls them, it doesn't own them.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `bundle.rs` | `Bundle` | Build the JSON shape emitted by `comemory context` |
| `code_prior.rs` | `RANK_SCALE` | The four bounded code priors: PageRank, ACT-R activation, working-set affinity, Beta feedback |
| `code_ref_collect.rs` | `RawRef` | Collect a memory's walked code-reference edges into resolved refs |
| `code_ref_fetch.rs` | `RefStatusCache` | Per-repo current-state lookups behind code-ref freshness |
| `code_ref_status.rs` | `RefStatus` | Freshness (`fresh\|stale\|ghost\|unpinned\|unknown`) classification of pinned refs |
| `code_rerank.rs` | `WORKING_SET_COMMITS` | Code rerank: relevance × the four `code_prior` boosts, chunk→parent coalesce |
| `code_route.rs` | `CodeRoutedHit` | Candidate stage for code search: weighted BM25 + thresholded ANN, RRF-fused |
| `code_search.rs` | `search_code_hits` | Shared code-search entry point (route → rerank) for `search-code` and `serve` |
| `diversify.rs` | `diversify` | SimHash near-dup collapse then MMR with token-set Jaccard similarity |
| `doc_route.rs` | `DocHit` | Document retrieval leg: BM25 over `document_fts`, chunk→parent coalesce |
| `fuse.rs` | `RankedHit` | Reciprocal Rank Fusion across ranked lists |
| `graph_route.rs` | `ALLOWED_RELS` | Graph-expansion leg: recursive-CTE walk from provisional top hits, fused as a third RRF list |
| `unified.rs` | `find` | `comemory find`'s entry point and the one-pool/one-paginate rule; weighted fusion lives in `unified/fuse_domains.rs` |
| `pipeline.rs` | `SearchOptions` | End-to-end memory search: route → rerank → diversify → top-k + access tracking |
| `rerank.rs` | `MEMORY_RANK_SCALE` | Multiply fused relevance by activation × feedback × quality × supersede × rank priors |
| `router.rs` | `CANDIDATE_POOL` | Route to vector, lexical, or hybrid path; the 4-tier lexical fallback ladder |
| `scope.rs` | `TimeScope` | Created-date window (`--since`/`--until`/`--as-of`) shared by every leg |
| `score.rs` | `activation` | Deterministic scoring primitives: ACT-R activation, Beta-smoothed feedback |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/retrieval.rs` (`pub mod
<name>;`) and callers import concrete paths.
