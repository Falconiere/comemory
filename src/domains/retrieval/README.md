# domains/retrieval/

**What belongs here:** the retrieval capability (#171) — the hybrid ranking
pipeline end to end plus the command cores built on it. Candidate
routing (`router`, `code_route`, `doc_route`), fusion (`fuse`), reranking by
bounded deterministic priors (`rerank`, `code_rerank`, `code_prior`, `score`),
diversification (`diversify`), graph expansion (`graph_route`), the
`--since`/`--until`/`--as-of` scope shared by every leg (`scope`), the
`comemory context` bundle shape (`bundle`), pinned code-reference freshness
(`code_ref_collect`/`code_ref_fetch`/`code_ref_status`), and the shared
code-search entry point (`code_search`) used by both `search-code` and
`serve`. On top of those sit the six cores both delivery adapters call —
`search`, `search_code`, `context`, `find`, `suggest` and the console-only
`config_retrieval` — the result models and `--json` envelopes both transports
serialize (`search_result`, `code_search_result`, `context_result`), and the
explain strip derived from a hit's `score_parts` (`explain`). Since #213 there
is one optional fourth ranking stage: `learned_rerank` reorders a fixed leading
prefix of the deterministic ranking through an out-of-process scorer when
`[rerank]` is enabled, `learned_report` is what it tells the caller, and
`staged` is the pause point that lets `serve` release its shared connection
mutex across inference.

**What does NOT belong here:** raw SQL table/DDL access beyond what a
candidate leg needs to build its ranked list. FTS5, `vec0`, and row CRUD
primitives live in `store/`; this capability calls them, it doesn't own them.
Nor delivery: clap `Args`, TTY colouring and `--json` emission stay in `cli/`
and its `output/` writers, and HTTP status mapping stays in `serve/routes/`.
No file here may import `cli` (the `cli::output` writers included) or `serve`.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `bundle.rs` | `Bundle` | Build the JSON shape emitted by `comemory context`; `assemble` takes `RankedMemory` (id + the pipeline's `final_score`) so each memory row's `score` is the real ranking number |
| `code_prior.rs` | `RANK_SCALE` | The four bounded code priors: PageRank, ACT-R activation, working-set affinity, Beta feedback |
| `code_ref_collect.rs` | `RawRef` | Collect a memory's walked code-reference edges into resolved refs |
| `code_ref_fetch.rs` | `RefStatusCache` | Per-repo current-state lookups behind code-ref freshness |
| `code_ref_status.rs` | `RefStatus` | Freshness (`fresh\|stale\|ghost\|unpinned\|unknown`) classification of pinned refs |
| `code_rerank.rs` | `WORKING_SET_COMMITS` | Code rerank: relevance × the four `code_prior` boosts, chunk→parent coalesce |
| `code_route.rs` | `CodeRoutedHit` | Candidate stage for code search: weighted BM25 + thresholded ANN, RRF-fused |
| `code_search.rs` | `search_code_hits` | Shared code-search entry point (route → rerank) for `search-code` and `serve` |
| `code_search_result.rs` | `SearchCodeResult` | The owned value `retrieval::search_code::run` returns, plus the `hits` envelope both delivery surfaces serialize |
| `config_retrieval.rs` | `UpdateRequest` | `GET|PUT /config/retrieval`: project and validate-then-write the live ranking knobs |
| `context.rs` | `Request` | `comemory context` / `GET|POST /context`: the shared middle |
| `context_result.rs` | `ContextResult` | The owned value `retrieval::context::run` returns, plus the flattened-bundle envelope both delivery surfaces serialize |
| `diversify.rs` | `diversify` | SimHash near-dup collapse then MMR with token-set Jaccard similarity |
| `explain.rs` | `ExplainPart` | The explain strip: a hit's `score_parts` as `{name, value, share, note}` rows, `share` being the log-magnitude partition of the multiplicative priors |
| `doc_route.rs` | `DocHit` | Document retrieval leg: BM25 over `document_fts`, chunk→parent coalesce |
| `find.rs` | `Request` | `comemory find` / `GET|POST /find`: the shared middle over the unified pipeline |
| `fuse.rs` | `RankedHit` | Reciprocal Rank Fusion across ranked lists |
| `graph_route.rs` | `ALLOWED_RELS` | Graph-expansion leg: recursive-CTE walk from provisional top hits, fused as a third RRF list |
| `unified.rs` | `find` | `comemory find`'s entry point and the one-pool/one-paginate rule; weighted fusion lives in `unified/fuse_domains.rs`. `run_legs` is `find` minus fusion and pagination, returning each leg's own rows with their passage text and version anchors intact, for the offline benchmark |
| `learned_report.rs` | `LearnedOrdering` | What the optional learned ordering stage did to one requested search — the `learned` key on every retrieval envelope |
| `learned_rerank.rs` | `LearnedStage` | The one optional learned ordering stage: materialize candidates, score a fixed prefix out of process, reorder it and preserve the tail |
| `pipeline.rs` | `SearchOptions` | End-to-end memory search: `rank` (route → rerank → diversify), `complete` (paginate + access tracking), `candidate_pool` |
| `staged.rs` | `Staged` | The pause point between a deterministic ranking and its learned order, so a shared connection lock is never held across inference |
| `rerank.rs` | `MEMORY_RANK_SCALE` | Multiply fused relevance by activation × feedback × quality × supersede × rank priors |
| `router.rs` | `CANDIDATE_POOL` | Route to vector, lexical, or hybrid path; the 4-tier lexical fallback ladder |
| `scope.rs` | `TimeScope` | Created-date window (`--since`/`--until`/`--as-of`) shared by every leg, `scope_from_flags` (which builds it from the three raw flag values, parsing each through `utilities::when::parse_when`), the `Domain`/`Domains` scope with its transport-neutral `resolve_domains` policy, and the `ScopeEcho` both envelopes flatten |
| `search.rs` | `Request` | `comemory search` / `GET|POST /memories/search`: the shared middle |
| `search_code.rs` | `Request` | `comemory search-code` / `GET|POST /code/search`: the shared middle |
| `score.rs` | `activation` | Deterministic scoring primitives: ACT-R activation, Beta-smoothed feedback |
| `search_result.rs` | `SearchResult` | The owned value `retrieval::search::run` returns, plus the `hits` envelope both delivery surfaces serialize and the shared `source_label` vocabulary |
| `suggest.rs` | `Request` | `GET /search/suggest`: mined expansions + recent queries |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/domains/retrieval.rs`
(`pub mod <name>;`) and callers import concrete paths. `unified/` has its own
[README](unified/README.md).
