# CLAUDE.md

## Project Overview

`comemory` is a Rust CLI that fuses engram-style developer memory, grepai-style
semantic code search, and ast-grep AST patterns into a single binary, knit
together by a SQLite-backed store (memory + code rows + edges). It is a
**standalone agentic-RAG toolbox** invoked from the command line — not a
Claude Code MCP plugin or in-process LLM. Everything runs locally: markdown
is the source of truth and one SQLite file (`comemory.db`) backs FTS5 +
`sqlite-vec` + edges.

## Architecture

- **Source of truth:** markdown files with YAML frontmatter at
  `~/.comemory/memories/{id}-{slug}.md` (override with `COMEMORY_DATA_DIR`).
- **Single SQLite file:** `~/.comemory/comemory.db` with `memories`,
  `memory_fts` (FTS5), `memory_vec` (`sqlite-vec` `vec0`), `code_symbols`,
  `code_fts`, `code_vec`, `edges`, `schema_meta`, plus stats / repo-marker
  tables. `rusqlite 0.32` with `bundled` + `load_extension` features.
- **Edges:** flat `(src_kind, src_id, edge_kind, dst_kind, dst_id)` rows
  (plus an integer `weight`) in the `edges` table replace the v0.1 kuzu
  graph. v6 adds code-graph kinds: `co_changed` (mined from git history)
  and `imports` (per-language import resolution), feeding a materialized
  PageRank on `code_symbols.rank_score`. Recursive CTEs handle multi-hop
  traversal.
- **AST extraction:** `ast-grep-core 0.38` + `ast-grep-language 0.38` (rust,
  typescript, javascript, python, go only).
- **Vectors are BYO.** No in-process embedder. Callers pass vectors via
  `--vector` (CSV) or `--vector-stdin` (JSON `{"embedding":[..]}`). A sample
  Ollama wrapper ships in `scripts/comemory-embed.sh`.
- **Output:** TTY via `owo-colors`, JSON via `serde_json`. Exit codes follow
  `sysexits.h`.
- **No in-process LLM.** All ranking is deterministic (RRF fusion of FTS5 +
  `sqlite-vec`, a tiered lexical fallback ladder ending in mined learned
  expansions, edge walks).

## Key Commands

```bash
cargo install --path .          # build + install the binary locally
just check                      # umbrella gate (alias of scripts/check-all.sh)
just test                       # cargo nextest run --all-features
just qa                         # check-all + cargo-deny + dup-check + machete
just e2e                        # real-binary end-to-end harness
bash scripts/check-all.sh       # the umbrella gate (CI parity)
bash scripts/guardrails/run.sh  # structure gate alone (file size, folder tree,
                                 # no mod.rs barrels, filenames, secrets, the
                                 # ast-grep pattern rules, folder READMEs)
cargo nextest run --all-features
comemory doctor                    # runtime health check (checks[] + scalars)
comemory stats                     # corpus counters + comemory.db size
comemory repos                     # indexed code repos + index freshness
comemory show <id>                 # one memory in full (body, activation, refs)
comemory search-code "query"       # ranked code search (BM25 + graph priors)
comemory find "query"              # one ranked list over memory + code + documents
comemory hooks                     # report/toggle the git reindex hooks
comemory edges "query"             # search the relation graph lexically
comemory eval                      # score retrieval (recall@k, MRR) vs golden set
comemory eval --history            # past eval/tune/bandit runs, newest first
comemory mine --apply              # distill query reformulations into expansions
comemory tune --apply              # grid-search ranking knobs into config.toml
comemory consolidate               # advisory near-duplicate cluster report
```

## Binding Rules (apply to every contribution)

comemory follows the toolu-conventions Rust stack
(`github.com/Falconiere/toolu-conventions`). These rules are that kit's, plus
comemory's stricter local ceilings. Deviations are enumerated under
"Deviations from toolu-conventions" — nowhere else.

1. **No duplication / redundancy.** Shared logic is extracted into a helper.
   Enforced by `scripts/dup-check.sh` (which excludes test trees) and review.
2. **No barrels — no `mod.rs`.** A module that grows into a folder keeps its
   file beside it: `src/store.rs` declares `mod migrate;`, `src/store/migrate.rs`
   holds it. A file whose only content is `pub use` re-exports is a barrel and
   is banned; `pub use` is legitimate only in `src/lib.rs`, to shape the crate's
   public API. Enforced by the `no-barrels` guardrails check
   (`barrelNames: ["mod.rs"]`).
3. **One responsibility per file; filename matches content.** `snake_case`,
   named after the file's primary item (`code_row.rs` holds
   `struct CodeSymbolRow` and its writers). Enforced by the `filename-case`
   guardrails check.
4. **Size ceilings.** 300 code lines per file in `src/` (blanks and comments
   excluded; tests exempt) — stricter than the kit's 500 by choice, because
   comemory's module decomposition is built around it. 100 lines per function.
   Both are DECLARED in `guardrails.config.json` (`fileSize.max`,
   `functionSize.max`) and enforced by the `file-size` guardrails check and
   `clippy::too_many_lines = "deny"` respectively.
5. **Zero errors, zero warnings, no silencing.** Clippy over all targets and all
   features with `-D warnings` must be clean. No `#[allow(...)]` in production
   code — a lint is either the house policy (declared once in
   `Cargo.toml [lints]`) or it is fixed. No `.unwrap()` / `.expect(...)` /
   `panic!` / `todo!` / `unimplemented!` / `println!` / `eprintln!` / `dbg!`
   outside tests and benches — return `Result` and propagate with `?`, and route
   diagnostics through `tracing`. Every `unsafe` block and every `unsafe fn`
   carries a `// SAFETY:` line in the comment block directly above it. Enforced
   by `Cargo.toml [lints]` plus the two project-local ast-grep rules in
   `scripts/guardrails/patterns/rust/`.
6. **Tests never share a file with production logic, and colocate by default.**
   No `#[cfg(test)] mod tests { ... }` body ever appears in a `src/` file. See
   "Testing" for the placement rule. Enforced by the `no-inline-test-module`
   ast-grep rule.
7. **Doc line on every module and public item.** `//!` at the top of every
   module, `///` on every `pub` item. Enforced by `missing_docs = "warn"` under
   `-D warnings`.
   A **folder** README is a separate rule and applies to folders only:
   `src/<module>/README.md` indexes the files inside `src/<module>/`. A
   single-file module — `src/store/memory_purge.rs`, `src/config/patch.rs`,
   `src/graph/neighbors.rs` — has no folder and therefore needs no README of
   its own; it is listed in its parent folder's README, and its `//!` doc is
   its documentation. `guardrails.config.json`'s `src.requireReadme` names
   the folders, and `scripts/guardrails/run.sh` is what enforces it.
8. **Docs in sync.** A change to a user-facing surface (CLI flags, public API,
   config, env vars) updates `README.md`, `CLAUDE.md`, `docs/` and the module's
   own `src/<module>/README.md` in the same change.
   `scripts/cli-docs-check.sh` enforces the `docs/cli-reference.md` half
   mechanically.
9. **Real data, no mock-data tests.** A test that only proves a mock returns
   what the mock was told to return is banned — it hides integration breakage.

The one command that must be green before every push:

    bash scripts/check-all.sh      # or: just check

## Code Style

- `rustfmt` stable-only options — **4-space indent**, 100-column line length,
  edition 2024 (`rustfmt.toml`). Nightly-gated knobs (`imports_granularity`,
  `group_imports`, `wrap_comments`, ...) are deliberately omitted so `cargo
  fmt` is deterministic on the stable toolchain CI uses.
- **≤300 code lines per `src/` file** (blanks/comments excluded; see
  Binding Rule 4) — split into submodules before crossing it.
- **No `mod.rs`.** A module that outgrows one file is `src/<name>.rs` beside
  `src/<name>/`, never `src/<name>/mod.rs`.
- **One primary item per file, filename matches content.** A file named after
  a type or function holds that item and its direct helpers, not an unrelated
  second concern.
- Lint policy is declared once, in `Cargo.toml [lints.rust]` / `[lints.clippy]`
  plus `clippy.toml` — never a per-call-site `#[allow(...)]`. Run it with
  `cargo clippy --all-targets --all-features -- -D warnings`
  (`scripts/lint-check.sh`).
- Doc comments (`///`) on every public item, `//!` at the top of every module.
- `Result<T>` alias from `crate::prelude::*`; errors flow through the
  `Error` enum in `src/errors.rs`.
- Use `tracing` for diagnostics, never `println!` / `eprintln!`.

## Module Map

Every folder listed with a trailing `/` below carries its own
`src/<module>/README.md` — the per-file index for that folder, kept current by
whoever last touched a file there. The table below is the cross-module
narrative; the folder `README.md` is the authoritative file-by-file list.

| Module | Responsibility |
|--------|---------------|
| `cli/` | clap subcommand entry points + the top-level dispatcher in `cli.rs`, plus `when` — the shared date-flag layer (`parse_when` for one `--since`/`--until`/`--as-of` value, `scope_from_flags` for the whole trio → a validated `TimeScope`), used by both `search` and `context`; `edges` is the fourth free-text surface (`comemory edges <query>` — lexical search over `edge_fts`, self-healing an empty index on first use) |
| `cli/graph/` | node assembly for `comemory graph`: `nodes` owns `NodeRow`, the `code_symbols` aggregate (including the `memories` citation count and the `blob` OID the console's selected-node panel shows), and `build_graph`. Split out when the donor `cli/graph.rs` hit the 300-line ceiling |
| `tui/` | read-only interactive terminal explorer (`comemory tui`): ratatui front end + async `EventStream`/`tokio::select!` loop (`tui.rs`), pure state (`app`) + key map (`event`), a dedicated-thread DB-worker that owns the connection (`worker`), the lexical/semantic request bridge (`search`), preview text (`preview`), RAII terminal guard (`terminal`), and pure ratatui widgets (`view/`). Embed shell-out lives in the shared `embed.rs` module |
| `embed.rs` | shared embed-command shell-out (single-file module, no children) — runs `COMEMORY_EMBED_CMD` / `--embed-cmd` as `sh -c <cmd>`, feeds the query on stdin, parses `{"embedding":[..]}`. Consumed by `tui` (Ctrl-S semantic enrich) and `serve` (`POST /api/v1/doctor/reembed`, the server-side re-vectorizing job; `GET /health` reports `embed_cmd_configured`) |
| `api/` | shared command core between `cli::` and `serve::routes::`: `api::<cmd>::run(&mut Ctx, Request)` holds each subcommand's logic — a *move* of each `cli::<cmd>::run`'s middle (arg-parsing and TTY/`--json` rendering stay in `cli::`), so neither surface duplicates it (reuse precedent: `retrieval::code_search::search_code_hits`, generalized here to every command). Every `Request` derives `#[serde(deny_unknown_fields)]`, enforced by `tests/api__parity.rs`'s clap-introspection walk. `Ctx` bundles `Paths` + `Config` with a connection that is either `Borrowed` (the CLI's own connection, or the server's shared per-request one) or `Lazy` (opened on first `Ctx::conn()` call — a job worker's own dedicated connection; conn-free commands like `doctor`, `rebuild`, `ast`, `install-hooks`, `completions` never open one at all). One file per subcommand (`api::save`, `api::search`, `api::list`, …) plus the console-compat surfaces `api::stats` / `api::repos` / `api::show` / `api::find` / `api::hooks`, plus five directory modules whose donor files sat near the 300-line ceiling: `graph/`, `index_code/` (+ `walk`), `doctor/` (+ `checks`, one function per health probe), `repos/` (+ `git_state`, the HEAD/remote/branch/changed-file probes, which DEGRADE to `status: "unknown"` and never propagate a git failure), `rebuild/` (+ `copy` and `documents`, the code-index/learning-state/document-domain ATTACH-copy run before the atomic DB swap, which itself snapshots the still-live DB to `comemory.db.pre-rebuild.bak` first). cwd-dependent middles (`save --ref-*` anchoring, the code rerank's working-set prior) resolve against the *calling process's* cwd — the server's cwd over HTTP, not the HTTP client's, documented behavior rather than a bug. The console-only cores (no CLI subcommand of their own, reached only through `serve::routes`) sit beside them: `overview`, `suggest`, `update`, `restore`, `refresh_refs`, `trash`, `graph_nodes`, `graph_recompute`, `index_runs`, `repo_admin`, `learning`, `learning_proposals`, `config_retrieval`, `reembed`, `gc_policy`, `memory_store` |
| `serve/` | loopback `/api/v1` HTTP server (`comemory serve`, API-only — the embedded web viewer was removed): axum `router` (mounts `routes::v1_router` behind the path-aware `guard` middleware — enveloped `401`/`403` JSON on `/api/v1/*`, plain text on any other `/api/*` path — and the 5 MiB `BODY_LIMIT`), `scope` (`RepoScope` — the per-request default `repo` filter: `X-Comemory-Repo` header first, the server's `--repo` second, never overriding an explicit parameter), `repo_root` resolution (also used by `retrieval::code_ref_fetch`), `security` (session token generation/matching, the loopback Host guard, `resolve_within` for repo-relative ids, and `contain_abs` — canonicalize-and-contain for the `/api/v1` mutating routes that take a raw filesystem path), `envelope` (the `{ok,data,meta}` / `{ok,error,meta}` `/api/v1` response envelope plus the one `Error → (StatusCode, code)` mapping table every HTTP error and every failed job's `{code, message}` derives from, with an optional structured `error.details`). `routes/` and `jobs/` are documented in their own rows below |
| `serve/routes/` | the versioned `/api/v1` REST surface: `routes.rs` aggregates every resource's `table_entries()` into one route table (method/path/CLI-command/`mutating` flag — the source of truth for the read-only gate, `GET /commands`, and `tests/api__parity.rs`) and owns the handler-layer helpers every resource shares — `run_blocking` (runs `api::<cmd>::run`, and the connection-mutex guard it takes, entirely inside one `spawn_blocking` closure, never across an `.await`), `respond`/`accepted` (envelope a result / a job-acceptance), `guard_mutating` (read-only-then-write-permit gate for a synchronous mutating route: `405 read_only`, else `503 busy` + `Retry-After` on permit contention), `guard_job` (read-only-only gate for a job-creating route — it always answers `202` immediately, permit contention only delays the job itself), `require_confirm` (the `confirmation_required` gate; its doc comment states the read-only-outranks-confirm ordering, AC-19), `track_for` (shared access-tracking suppression for `search`/`search-code`/`context`). Per-resource files: `memories/` (`memories.rs` — `GET /memories`, `GET /memories/{id}`; `search.rs` — `GET|POST /memories/search`, `GET|POST /context`; `write.rs` — `POST /memories`, `DELETE /memories/{id}?confirm`, `POST /feedback`), `code.rs` (`GET|POST /code/search`, `POST /code/ast` with pre-run containment, job-backed `POST /code/index` and `POST /code/ingest` under its own 64 MiB body-limit layer), `graph.rs` (`GET /graph`/`GET /edges`, reusing the legacy `build_code_graph`/`build_graph_page` pair — no second query path), `sources.rs` (`GET /sources`, job-backed `POST /sources`, `DELETE /sources?target=&confirm=`), `learning.rs` (job-backed `POST /eval` read-class, `POST /tune`/`POST /bandit` confirm-gated only when `apply`, `golden` containment before every other check), `maint/` (`maint.rs` — `GET /doctor`, `GET /consolidate`; `prune.rs` — `GET|POST /prune`, `POST /gc`, plus `split_confirm` — the shared raw-body confirm-field extractor every confirm-gated route with a real `Request` type reuses; `admin.rs` — `POST /mine`, `POST /hooks/install`, job-backed `POST /rebuild` + the shared-connection swap), `meta.rs` (`GET /completions`, `GET /commands` — the clap-introspected route/command inventory), `stats.rs` (`GET /stats`), `repos.rs` (`GET /repos`), `find.rs` (`GET|POST /find` — its own resource because it is cross-domain, not a memories sub-resource), `hooks.rs` (`GET /hooks` read, `POST /hooks` per-hook toggle behind the read-only gate; NOT confirm-gated, since writing a hook file is idempotent and reversible), `jobs.rs` (`GET /jobs`, `GET /jobs/{id}`, `GET /jobs/{id}/events` SSE with `status`/`progress`/`log` events, `POST /jobs/{id}/cancel`). The console-api spec (2026-09-01, `docs/toolu/specs/2026-09-01-console-api-design.md`) added, as additional flat resource files: `overview.rs`, `search.rs` (the console view over `find` + suggest + per-hit feedback), `trash.rs`, `graph_nodes.rs`, `index_runs.rs`, `repos_admin.rs`, `learning_console.rs`, `config.rs`, `memory_stores.rs`, plus `memories/edit.rs` (PATCH/restore/refresh) and `maint/{doctor,gc}.rs` — see `src/serve/routes/README.md` for the per-file route list |
| `serve/jobs/` | the background job model for long-running commands (`index-code`, `ingest-code`, `index`, `rebuild`, `eval`, `tune`, `bandit`, `graph-recompute`, `reembed`, `gc`, `store-sync`): `registry` (`Arc<Mutex<HashMap<JobId, Job>>>` plus one retained `watch::Sender<JobStatus>` per job, so a late SSE subscriber's `borrow_and_update()` still replays a terminal status; a per-job `broadcast` log channel behind the SSE `log` event; a per-job cooperative cancel flag — `cancel(id)` marks a queued job `Cancelled` outright and asks a running one to stop at its next `ProgressSink::is_cancelled` boundary; `active_for(command, repo)` — the liveness check behind `409 index_running`; finished jobs beyond the 100 most recent are evicted on every insertion; job ids are 8 random `/dev/urandom` bytes, the same entropy source as the session token at a shorter width), `worker` (`spawn_job` — registers the job `Queued`, then on its own `tokio::spawn` task awaits the single write permit FIFO for `mutating` jobs only (a read-class job like `eval` never touches it), marks `Running`, runs the caller's closure — typically `api::<cmd>::run` over `Ctx::lazy`, the job's own connection — on `spawn_blocking`, and records the terminal `Done`/`Error` status). `JobView` also carries `progress: Option<Progress>` and a bounded 20-line `log_tail`, surfaced as a SECOND SSE event type (`event: progress`) — the `status` event payload itself is deliberately unchanged — `JobStatus` gained only the terminal `Cancelled` variant, so an existing client's `queued`/`running`/`done`/`error` events stay byte-identical. `events` holds the three SSE payload types. Lifecycle is `Queued → Running → Done \| Error \| Cancelled`, not persisted: a server restart forgets every unfinished job |
| `memory/` | markdown I/O, `Frontmatter`, slug, id (8-hex SHA-256), atomic save / load / soft-delete / list |
| `document/` | pure, in-process document extraction (TXT/Markdown/HTML/CSV) and size-bounded chunking, independent of the store — `extract`/`html`/`delimited` (format-specific extractors), `chunk` (the shared paragraph-boundary splitter), `fingerprint` (size+mtime skip check, SHA-256 identity), `writer` (the per-file index writer: fingerprint skip → extract → one-transaction row replacement) |
| `source/` | durable source registry (`sources.toml`): `registry` (load/save, overlap validation, atomic durability), `lock` (exclusive flock guard over concurrent read-modify-write), `discover` (the boundary/ignore-rule walk over a registered root), `classify` (extension allowlist + binary sniff), `mirror` (reconciles the TOML registry into SQLite's `source_roots`) |
| `store/` | central SQLite layer — `connection` (pooled rusqlite + `sqlite-vec` loader), `schema`, `migrate` (versioned + idempotent, applying the `MIGRATIONS` slice declared in `migrate/list.rs`; DDL text in `sql/`; `migrate/preflight` + `migrate/backup` are the forward-compat guard and pre-upgrade `VACUUM INTO` snapshot run from `connection::open` before the chain — see `docs/guides/upgrading.md`), `vector` (`vec0` insert/KNN with dim guard), `fts` (FTS5 helpers, code leg), `fts_memory` (the memory FTS ladder behind one `run_memory_match` choke-point — every tier inherits the same filters), `CreatedWindow` in `store.rs` (the borrow-only `{since, cutoff}` pair each SQL predicate takes, compared via `datetime()`; keeps `store/` free of `retrieval/` types), `embed` (`to_vec_blob`, dim helpers), `edge_fts` (FTS5 triplet index over `edges` — per-kind `src —rel→ dst` rendering, wholesale refresh-materialize in one tx, `needs_refresh` for the upgrade self-heal, and the two-tier strict→word-OR ladder behind `comemory edges`), `memory_meta` (`fetch_meta` — batched per-memory metadata: path/repo/kind/tags/references backing the enriched `search --json` rows), `memory_row`/`code_row` (the per-table mirror-row upserts), `memory_list` (paginated memory listing, `--sort created|quality|accessed`), `eval_runs` / `gc_runs` / `index_runs` (the v14/v15 run-history writers + readers; all three are in `rebuild`'s `COPIED_TABLES` — history is not reconstructable from markdown), `repo_drop` (`DELETE /api/v1/repos/{name}`: drop every code-index row and file edge for one repo label in one transaction, memories kept), `random_id` (the shared random-hex id helper, moved out of `serve::security` so non-HTTP callers can use it), `code_ref` (the version-anchor side table for explicit code references), `documents`/`document_fts` (the document/chunk mirror + its BM25 leg), `sources` (the SQLite mirror of `source::registry`), `simhash_scan` (bulk fingerprint scan shared by save + consolidate), `tokenizer/` (custom FTS5 identifier tokenizer: camelCase/snake_case split + FFI registration) |
| `retrieval/unified/` | `comemory find`'s core: the three legs (`router`, `code_route`, `doc_route`) run unchanged and their *reranked* orders fuse via the pre-existing `fuse::rrf_multi_weighted`, memory and code at weight 1.0 and documents at `retrieval.document_leg_weight` (declared and validated since the document domain landed; read by nothing until this module). One shared `pipeline::pool_size` across every leg and ONE `pipeline::paginate` over the fused list — RRF is prefix-stable, so divergent per-leg pools would let a deeper page reorder a shallower one. `fuse_domains` owns the weighted fusion and `UnifiedHit`/`HitParts`, the untagged enum carrying each domain's own `score_parts` verbatim |
| `simhash.rs` | 64-bit SimHash + Hamming distance over tokenized memory bodies (siphasher-based) |
| `index.rs` | intentionally empty placeholder — v0.1's LanceDB/fastembed/tantivy indexing lived here; v0.2 moved it into `store::vector`/`store::fts`, and the module stays so `comemory::index` remains a stable path for any future re-introduction |
| `graph/` | SQL-backed `edges` table upserts, recursive-CTE walks, `cross_link` reference extraction, `cochange` (git-history co-change mining), `imports` (per-language import edges), `pagerank` (deterministic weighted PageRank), `materialize` (writes `rank_score` onto `code_symbols`), `memory_rank` (the same PageRank over the memory graph — direct memory→memory relations plus in-memory co-citation edges, hub rels excluded — written onto `memories.rank_score`), `coactivate` (commit co-activation reward: a commit touching a memory's referenced files reinforces it), `doc_link` (deterministic `member_of_source`/`references_document` link deriver), `search_edit` (search→edit lookback feeding `auto_search_edit` provenance), `derived` (`refresh_derived_best_effort` — the single post-write pass that refreshes *both* derived artifacts, `memories.rank_score` and the `edge_fts` triplet index, independently best-effort; called at the four seams `save`, `delete`, `rebuild`, and `index-code`), `neighbors` (the one-hop undirected `imports`/`co_changed` file neighborhood query shared by `retrieval::bundle` and `GET /api/v1/graph/nodes/{id}/neighbors`) |
| `retrieval/` | `router` (candidates + 4-tier lexical ladder: strict → word-OR → subtoken-OR → tier-4 learned expansion from mined `query_expansions`), `doc_route` (the document-search leg: BM25 over `document_fts`, chunk→parent coalesce), `scope` (`TimeScope` — the `--since`/`--until`/`--as-of` created-date window plus its as-of supersede semantics — and `Filters`, the `{repo, kind, scope}` bundle every leg narrows candidates by), `score` (ACT-R/Beta scoring primitives), `rerank` (five multiplicative priors over the max-normalized relevance: activation × feedback × quality × supersede × rank, the last being the pool-median-relative `memories.rank_score` boost), `diversify` (SimHash near-dup collapse + MMR), `pipeline` (orchestration + access tracking), `fuse` (RRF: pairwise `rrf_k` + N-ary `rrf_multi`), `graph_route` (graph-expansion leg — one undirected recursive-CTE walk over `edges` from the provisional top hits, fused in as a third RRF list; `graph_hops = 0` or an empty expansion returns the provisional ranking untouched), `bundle` (context lookup with graph-prior-ranked code refs), `code_route` (code candidates: BM25 + thresholded ANN + RRF, chunk→parent coalesce), `code_rerank` (four-prior code rerank), `code_prior` (PageRank / recency / working-set affinity / feedback priors), `code_search` (`search_code_hits` — the shared code-search entry point used by both the `search-code` CLI and `GET|POST /api/v1/code/search`), `unified/` (the `comemory find` core — see its own row), `code_ref_collect`/`code_ref_fetch`/`code_ref_status` (pinned code-reference freshness: collect a memory's walked ref edges, fetch per-repo current state, classify `fresh|stale|ghost|unpinned|unknown`) |
| `eval/` | learning loop — `golden` (YAML golden sets + feedback harvest), `metrics` (recall@k, MRR), `runner` (eval over the real pipeline, tracking off), `mine` (reformulation mining → `query_expansions`), `tune`/`tune_sample` (deterministic or seeded-sampled grid search over the blend knobs), `bandit`/`bandit_rng` (eval-gated Thompson-sampling bandit over the same `[tune]` grid, with a dependency-free SplitMix64 + Beta/Gamma sampler) |
| `ast/` | `extractor` (symbol enumeration via tree-sitter through ast-grep — rust/ts/js/py/go only), `chunk` (cAST split of oversized symbols into child rows at AST boundaries), `pattern` (user-facing `comemory ast`), `languages`/`pattern_cache` (per-language wiring and the process-global compiled-pattern cache) |
| `stats/` | usage / feedback / `code_feedback` (per-symbol counters) / repo-marker tables (lives inside `comemory.db`), plus `sqlite` (`StatsDb`, opened via the shared `store::connection`) |
| `config/` | layered config (defaults → `file` → `env`) and `paths::Paths` (data-dir layout), the `learning` section (`[tune]` grids, `[reinforce]`, `[bandit]`), the `retrieval` section (the `[retrieval]` knobs), `validate` (the shared invariant pass run after every layer is applied), and `patch` (`patch_config_file` — the one read-patch-atomically-write primitive over `config.toml`, shared by `tune --apply`, the `hooks` reinforce toggle, and the console's `PUT /config/retrieval` / `PUT /gc/policy` / `PATCH /memory-stores/{id}`; every writer validates the would-be `Config` in memory before touching the file) |
| `output/` | TTY (`owo-colors`) and JSON (`serde_json`) emitters, shared between subcommands, plus `page` (the generic `Page<T>` pagination envelope every paged command serializes) and `explain` (the console's explain strip: a hit's `score_parts` rendered as `{name, value, share, note}` rows, `share` being the log-magnitude partition of the multiplicative priors) |
| `prune/` | orphan / low-value / stale-code detection plus soft-delete & gc |
| `consolidate/` | advisory near-duplicate cluster report (`comemory consolidate`) — `cluster` (transitive union-find over the stored SimHashes within a radius, unfingerprinted rows dropped and counted), `keeper` (member metadata plus the quality → access → recency → PageRank → id keeper order, and in-cluster supersede resolution); `detect` + the `Member`/`Cluster`/`Scan` types live in `consolidate.rs`. Read-only end to end; the merge stays a human `save --supersedes` |
| `git_utils.rs` | repo + author auto-detection, blob OID lookup, git-hook installation helpers |
| `errors.rs` | `thiserror`-derived `Error` enum and `Result<T>` alias |
| `prelude.rs` | crate-internal prelude (`Error`, `Result`, common imports) |
| `lib.rs` / `main.rs` | library surface (carries `extern crate self as comemory;`, see Testing) + binary entry that parses `Cli` and calls `cli::run` |

## Environment Variables

Values are layered: defaults (`Config::defaults`) → optional config file →
environment (`Config::with_env`, in `src/config/env.rs`).

| Variable | Purpose | Default |
|----------|---------|---------|
| `COMEMORY_DATA_DIR` | Root data directory (`memories/` + `comemory.db`) | `~/.comemory` |
| `COMEMORY_INDEXING_AUTO_REINDEX` | `lazy` \| `hook` \| `off` — automatic code-index refresh. `lazy` (wired in `src/cli/lazy_reindex.rs`): `search-code`/`context` spawn a detached, non-blocking `index-code` when the repo HEAD moved since the last index, then search the current index immediately; `hook` relies on installed git hooks; `off` is manual-only | `lazy` |
| `COMEMORY_RETRIEVAL_TOP_K` | Number of results returned by the hybrid router (also the default page size for `search` / `search-code` / `context` when `--k`/`--limit` is omitted) | `12` |
| `COMEMORY_RETRIEVAL_MAX_PAGE_WINDOW` | Maximum depth pagination can reach into the ranked result list. `search` / `search-code` / `context` fetch a candidate pool sized `clamp(offset + k + k, CANDIDATE_POOL, max_page_window)`, run the full fuse → rerank → diversify pipeline over it, then slice `[offset, offset+k]`; `has_more` is forced false once this ceiling is hit (deeper results require refining the query). Validated `> 0`. | `200` |
| `COMEMORY_RETRIEVAL_MEMORY_THRESHOLD` | Minimum cosine similarity for the memory table | `0.55` |
| `COMEMORY_RETRIEVAL_CODE_THRESHOLD` | Minimum cosine similarity for the code table (ANN leg of `search-code`, range `[0.0, 1.0]`) | `0.50` |
| `COMEMORY_RETRIEVAL_RRF_K` | RRF fusion constant for hybrid scoring | `60.0` |
| `COMEMORY_RETRIEVAL_GRAPH_HOPS` | Maximum hop depth of the graph-expansion leg — the `edges` walk seeded from the provisional top hits of memory `search`. Validated `≤ 4`; `0` disables the leg and retrieval takes the legacy two-leg path unchanged | `2` |
| `COMEMORY_RETRIEVAL_GRAPH_SEEDS` | How many provisional top hits seed that walk. Validated `≥ 1` | `8` |
| `COMEMORY_RETRIEVAL_BM25_WEIGHTS` | `"body,tags"` BM25 column weights for `memory_fts` (both finite ≥ 0, at least one > 0) | `1.0,3.0` |
| `COMEMORY_RETRIEVAL_CODE_BM25_WEIGHTS` | `"symbol,snippet,path_tokens"` BM25 column weights for `code_fts` (all finite ≥ 0, at least one > 0) | `2.0,1.0,1.5` |
| `COMEMORY_RETRIEVAL_DOCUMENT_LEG_WEIGHT` | Weighted-RRF contribution of the document leg, relative to the memory and code legs (both fixed at `1.0`), in the default `search` order. Finite, in `(0.0, 10.0]`. Deliberately absent from the `[tune]` grids — `tune`/`bandit` pin memory-only scope, so a memory-only metric would drive this cross-domain knob toward zero | `0.5` |
| `COMEMORY_INDEXING_MAX_FILE_BYTES` | Ceiling (bytes) above which a candidate document file is recorded `too_large` and skipped by the `comemory index` document writer rather than extracted — plain-text formats past this size are logs, not documents. Must be > 0 | `16777216` (16 MiB) |
| `COMEMORY_LEARNING_RETENTION_DAYS` | `comemory gc` retention window (days) for raw `retrieval_log` + `feedback_events` rows; aggregated `feedback` counters and mined `query_expansions` never expire | `90` |
| `COMEMORY_TUNE_MIN_GOLDEN` | Test hook lowering `comemory tune` / `comemory bandit`'s minimum-golden-pairs floor; not a tuning knob | `10` |
| `COMEMORY_REINFORCE_SEARCH_EDIT_DAYS` | Lookback days for search→edit auto-reinforcement provenance (`auto_search_edit`). Validated `≥ 1` | `7` |
| `COMEMORY_DISABLE_ACCESS_TRACKING` | Test hook (truthy) disabling `search` / `context` access tracking + `retrieval_log` writes for one run, so a stability harness can drive the binary repeatedly without each query mutating `access_count` / `last_accessed` (which feeds ACT-R activation and reorders ranking between calls); not a user knob | `false` |
| `COMEMORY_GIT_AUTO_SYNC` | `true`/`1` to enable best-effort git commit + push after a save. Also a file key: `[git] auto_sync` / `[git] remote` in `config.toml` (written by `PATCH /api/v1/memory-stores/default`); env wins over file | `false` |
| `COMEMORY_EMBED_HINT` | Free-form identifier of the embedder you used (e.g. `ollama:nomic-embed-text`). Surfaced by `comemory doctor`; never consumed as a switch. | unset |
| `COMEMORY_EMBED_CMD` | Embed command used by `comemory tui`'s Memory-tab semantic enrich (Ctrl-S) **and** by `comemory serve`'s `POST /api/v1/doctor/reembed` (re-vectorize memories/code server-side). Run as `sh -c <cmd>`; reads the text on stdin, must emit `{"embedding":[..]}` on stdout. The per-command `--embed-cmd` flag (on `tui` and `serve`) overrides it. Unset → semantic enrich is a no-op / reembed answers `503 embedder_unavailable`; lexical search always works. | unset |
| `COMEMORY_RANK_DECAY` | ACT-R decay exponent `d` in `ln(n) − d·ln(days+1)`. Must be ≥ 0. Higher → older memories decay faster. | `0.5` |
| `COMEMORY_RANK_PRIOR_CLAMP` | `"lo,hi"` bounds applied to the activation, feedback, quality, and PageRank boost multipliers (the fixed `0.2` supersede penalty intentionally bypasses the clamp). Both finite; lo > 0, lo ≤ hi. | `0.5,2.0` |
| `COMEMORY_RANK_MMR_LAMBDA` | MMR relevance-vs-diversity trade-off in `[0.0, 1.0]`. `1.0` = pure relevance; `0.0` = pure diversity. | `0.7` |
| `COMEMORY_RANK_NEAR_DUP_HAMMING` | SimHash Hamming radius for near-dup detection (save-time advisory + diversify collapse). Must be ≤ 64 (SimHash is 64-bit). | `8` |
| `COMEMORY_PRUNE_MIN_ACTIVATION` | Activation floor (ACT-R scale) below which a memory is prune-eligible. | `-2.0` |
| `COMEMORY_PRUNE_MIN_FEEDBACK` | Beta-feedback ceiling (range `[0.0, 1.0]`) at or below which a memory is prune-eligible. | `0.25` |
| `COMEMORY_PRUNE_BELOW_QUALITY` | Quality threshold (1..=5); memories at or below this value are prune candidates (used together with activation + feedback floors). | `2` |
| `COMEMORY_PRUNE_SUPERSEDED_GRACE_DAYS` | Grace window (days) before a superseded-and-never-accessed memory becomes prune-eligible; protects freshly-rebuilt DBs whose supersede edges all carry rebuild-time timestamps. | `7` |
| `COMEMORY_SKIP_MIGRATION_BACKUP` | Truthy (`1`/`true`) skips the pre-migration `VACUUM INTO` snapshot (`comemory.db.pre-v{N}.bak`) that `store::migrate::preflight` otherwise takes before ANY pending schema migration — a failed snapshot only refuses the upgrade when a pending migration is destructive, and merely warns otherwise. See `docs/guides/upgrading.md`. | `false` |

`[reinforce] enabled` (default `true`) is file-only, with no env override:
it is the on/off bit behind `comemory hooks --enable|--disable
search-edit-reinforcement`. `COMEMORY_REINFORCE_SEARCH_EDIT_DAYS` still sets
the lookback window, but a window cannot express "off" (it is validated
`≥ 1`), which is why the flag is a separate boolean rather than a magic `0`.

The `[tune]` knobs are file-only — set them in `config.toml`; they have no
env override. Six grids (`tune.rrf_k_grid`, `tune.decay_grid`,
`tune.mmr_lambda_grid`, `tune.bm25_grid`, `tune.graph_hops_grid`,
`tune.graph_seeds_grid`) define the search space — 729 candidates at the
defaults — and `tune.samples` (default `64`) caps how many of them a run
actually scores by drawing that many distinct candidates from the pools with
a derived-or-`--seed`-pinned PRNG; `tune.samples = 0` restores the exhaustive
cartesian sweep. `comemory bandit` ignores `tune.samples` — its arms stay the
full grid.

The memory and code vector dims (1024 and 768) are baked into the
`memory_vec` / `code_vec` vec0 DDL (`src/store/sql/0002_v2_tables.sql`)
at migration time and are not env-configurable: a divergent env value
would silently disagree with the vtab and surface as `VecDimMismatch`
at first insert. Change the literal in the DDL if you need a different
dim.

CLI flags `--data-dir` and `--json` are global and can appear before or
after the subcommand.

## Memory Data Model

Frontmatter schema v1, defined by `src/memory/frontmatter.rs::Frontmatter`:

```yaml
---
id: a1b2c3d4                  # 8-hex prefix of SHA-256(body.trim_end())
kind: decision                # decision|bug|convention|discovery|pattern|note
repo: qwick-backend           # single repo string (not a list)
tags: [database, postgres]
author: falconiere
created: 2026-05-17T14:30:00Z
quality: 4                    # 1..=5, default 3
schema: 1
content_hash: <64-hex SHA-256 of body.trim_end()>
references:
  symbols: []                 # qualified symbol ids: <repo>:<path>:<name>
  files:   []                 # qualified file paths:  <repo>:<path>
relations:
  supersedes:     []          # memory ids this one replaces
  conflicts_with: []          # memory ids this one contradicts
  derived_from:   []          # memory ids this one builds on
---

Markdown body lives here.
```

## Save Flow (BYO-vector, current implementation)

`comemory save` runs:

1. Parse args, resolve repo/author defaults, build `Frontmatter` with
   `schema: 1` and `content_hash = sha256(body.trim_end())`.
2. If `--vector` (CSV) or `--vector-stdin` (JSON `{"embedding":[..]}`) is
   set, parse it into a `Vec<f32>` and run the `store::embed::dim_guard`
   against `schema_meta` so a mismatched embedder fails fast with
   `Error::VecDimMismatch`. With neither flag, the save is lexical-only —
   no `memory_vec` row is written.
2a. **Near-duplicate check** (best-effort, advisory): scan live `memories`
   rows for a SimHash Hamming distance within `NEAR_DUP_HAMMING`. If a
   near-duplicate is found its id is recorded as `duplicate_of`. The save
   always proceeds; the caller decides whether to re-save with
   `--supersedes <id>`. TTY
   mode prints a `warning: similar memory <id> exists` to stderr;
   `--json` mode includes `"duplicate_of": "<id>"` in the output object.
   Self-matches (re-save of the same body, same content-hash-derived id)
   are filtered out.
3. Atomic stage: write `memories/.{id}.tmp`, then `fs::rename` to
   `memories/{id}-{slug}.md`. On failure between stage and rename, the tmp
   file is removed.
4. Single `store` transaction:
   - upsert `memories` row (frontmatter + body + simhash)
   - upsert `memory_fts` row (FTS5)
   - upsert `memory_vec` row (`vec0`) when a vector was supplied
   - frontmatter `relations.{supersedes,conflicts_with,derived_from}` ids
     (`supersedes` is populated by `--supersedes`; the others come from
     hand-edited markdown) are materialized as memory→memory `edges` rows.
     Targets may dangle — every consumer (rerank's supersede penalty,
     prune's superseded-rule, `supersedes_chain`) joins on live `memories`
     rows. `comemory rebuild` re-materializes these edges from markdown.
   - `cross_link::extract_refs` walks the body for backtick-fenced
     `<repo>:<path>` / `<repo>:<path>:<symbol>` mentions and writes
     `ReferencesFile` / `ReferencesSymbol` rows into `edges`. Missing
     `code_symbols` rows are tolerated — `comemory index-code` fills them
     in later.
5. Best-effort git auto-sync via `git_utils`, only when
   `COMEMORY_GIT_AUTO_SYNC` is enabled.

If the SQLite mirror transaction fails, the markdown file is **kept** (it
was already written as the source of truth) and the error wraps the
markdown path with a hint to run `comemory rebuild`. `rebuild` fully
reconstructs the memory layer — rows, `memory_fts`, and edges — from
`memories/*.md`, and (snapshotting the still-live `comemory.db` to
`comemory.db.pre-rebuild.bak` first) preserves everything markdown cannot
rebuild — the code index, the document index, and the learning-loop
tables — by copying it from the pre-rebuild database rather than
regenerating it. Two things it does **not** do: repopulate `memory_vec`
rows (the BYO-vector contract means only the caller's embedder can do
that — re-run `comemory save` / `ingest-code`), and re-walk indexed repos
(the code/document index is carried across, not rebuilt from source). See
the README "BYO-Vector workflow" section, `scripts/comemory-embed.sh` for
the recommended caller pattern, and `docs/guides/upgrading.md` for the
schema-migration snapshot `rebuild` shares its mechanism with.

## Testing

- Runner: `cargo nextest run --all-features` (alias `just test`).
- **Test code never lives in a production file.** No `#[cfg(test)] mod tests { ... }`
  body in any `src/` file, ever.

### Where a test goes

Two homes, and one rule decides between them. Applied in order, first match
wins:

1. **It drives the real `comemory` binary** (`assert_cmd`) -> crate-root
   `tests/`. A subprocess consumer is the most real consumer there is, and
   `cargo_bin` resolution is only reliable for an integration target.
2. **It owns an `insta` snapshot** -> crate-root `tests/`. insta derives its
   snapshot directory from the test file's location; `tests/snapshots/` is the
   one reviewed home.
3. **It exercises the CLI surface** (`tests/cli*.rs`) -> crate-root `tests/`.
   The CLI is one public surface; its suite stays in one place.
4. **Otherwise** -> colocated, in a sibling `tests/` folder beside the module
   under test:

       src/store/migrate.rs          production module
       src/store/tests/migrate.rs    its tests
       src/simhash.rs                a crate-root module
       src/tests/simhash.rs          its tests

   The production file names its test files with a one-line include, which makes
   it the index of its own suite:

       #[cfg(test)]
       #[path = "tests/migrate.rs"]
       mod tests;

Keep each `tests/` tree **flat**. A suite that outgrows one file splits into
`<name>_2.rs` (or `<name>_v4.rs` for a version-scoped suite) beside it, and the
production file gains a second include (`mod tests_2;`). Tests are exempt from
the 300-line ceiling, so splitting is a readability choice, not an obligation.

Measured today (2026-09-01): 199 colocated `src/**/tests/*.rs` files, 92
crate-root `tests/*.rs` surfaces (54 CLI, 33 `assert_cmd`, 4 `insta`).

### Why crate-root `tests/` cannot be deleted

Rules 1 and 2 above are not stylistic preferences, and the question "why not
colocate *everything* and drop `tests/`?" recurs. It was measured on
2026-08-07; the answer is that three separate things anchor the directory.

**1. `CARGO_BIN_EXE_comemory` (the binding constraint).** `assert_cmd`'s
`Command::cargo_bin("comemory")` reads the `CARGO_BIN_EXE_comemory` env var,
which Cargo publishes **only to integration-test and bench targets**. From a
lib unittest it is unset and assert_cmd panics outright — its own help text
says *"if this is running within a unit test, move it to an integration test
to gain access to `CARGO_BIN_EXE_comemory`."*

There is a trap here worth knowing about. If `target/debug/comemory` already
exists, `cargo-nextest` sets the variable at runtime and a colocated
`assert_cmd` test passes — so on a warm target dir the constraint looks
absent. It is not. Delete the binary, or run `cargo nextest run --lib`, and
the same test panics. Verified both ways, and separately on a scratch crate
with no `tests/` directory at all, where `cargo nextest run` never built the
binary.

The consequence of removing the last integration target is therefore worse
than a build error: nothing would make Cargo rebuild the binary during a test
run, so the 32 `assert_cmd` surfaces would silently exercise **whatever stale
binary was left in `target/`** and report green against source you had already
changed. A false green is a worse failure mode than a red build, which is why
this is a rule rather than a preference.

If the directory is ever collapsed anyway, the cheapest way to keep the
guarantee is to leave exactly one integration target whose only job is to
force the binary build — not to drop to `cargo build --bin comemory && cargo
nextest run`, which fixes only the sanctioned entry points and leaves a bare
`cargo nextest run` silently stale.

**2. insta snapshot resolution.** insta derives its snapshot directory from
the test file's own location, so colocating the 4 snapshot owners would
scatter `snapshots/` directories under `src/` — which the `folder-tree`
guardrail rejects (`src.nested` allows only `tests` and
`proptest-regressions`). It would need either a new allowlist entry or a
redirected `Settings::set_snapshot_path`, and `tests/snapshots/` stays the one
reviewed home instead.

**3. Fixture and data files.** `tests/common/` holds the single copy of every
shared fixture (D9), and `tests/golden/`, `tests/ast/fixtures/` and
`tests/common/fixtures/` hold data addressed through
`concat!(env!("CARGO_MANIFEST_DIR"), "/tests/...")`. These are the movable
part — `tests/common/*.rs` could become `src/test_common/*.rs` and the data
could move to a top-level `fixtures/` — but only in a world where constraint 1
is already solved, because `#[cfg(test)]` items in `src/` are invisible to an
integration-test binary.

### Conventions inside a test file

- First line, always:

      #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp, clippy::too_many_lines)]

  A failed assertion is how a test reports; unwrapping is the reporting
  mechanism, not a bug. This is the toolu Rust template's own header plus the
  three lints that only ever fire in test code here.
- Import through the crate name — `use comemory::store::migrate::{...};` — in
  **both** homes. `src/lib.rs` carries `extern crate self as comemory;` so a
  test file reads identically wherever it lives, and tests keep exercising the
  public surface rather than ossifying internals. Reach for `use super::*;` only
  when a test genuinely needs a private item.
- Shared fixtures live in `tests/common/` and are reached from a colocated test
  through the bridge module: `use crate::test_common::git_repo;`. Crate-root
  tests keep including them directly with `#[path = "common/git_repo.rs"]`.
  There is exactly one copy of each fixture. The bridge itself is
  `src/test_common.rs` — a `#[cfg(test)]`-only module declared from
  `src/lib.rs` that re-`#[path]`s the eight fixtures colocated tests actually
  use; it is the migration's one sanctioned `#![allow(dead_code)]` under
  `src/`, exempted by name in `no-allow-attribute.yml`.
- Data files (`tests/golden/`, `tests/common/fixtures/`, `tests/ast/fixtures/`)
  stay at the crate root and are addressed with
  `concat!(env!("CARGO_MANIFEST_DIR"), "/...")`, which is stable no matter where
  the test file lives. Never a `../../..` relative path.
- Real data, real integration paths. No mock-data tests.

`.config/nextest.toml` serializes two groups: `embedder`
(`embedder|memory_index|code_index`), so the fastembed model download cannot
race, and `config::env::tests*`, because those tests mutate process-global env
vars and colocation put them in the crate's single lib-unittest binary
alongside every other colocated test (a `binary(...)` filter no longer
isolates them; a name filter does).

## Quality Gates

`bash scripts/check-all.sh` is the umbrella gate. It runs, in order:

```
scripts/fmt-check.sh             # cargo fmt --check
scripts/type-check.sh            # cargo check --all-targets --all-features
scripts/lint-check.sh            # cargo clippy --all-targets --all-features -- -D warnings
scripts/guardrails-check.sh      # scripts/guardrails/run.sh (see below)
scripts/typos-check.sh           # typos
scripts/cli-docs-check.sh        # docs/cli-reference.md vs the real --help output
scripts/migration-check.sh       # shipped src/store/sql/*.sql is byte-identical
                                  # to its content at the first release tag
```

Retired in the toolu-conventions migration, folded into the two gates above:
`test-placement-check`, `no-bypass-check`, `module-size-check`,
`tests-mirror-check`.

Gate ownership — one rule, one enforcer. `guardrails.config.json` is the
single declaration of every structural ceiling (file size, function size, the
nested-folder allowlist, required per-folder READMEs); nothing else declares
a competing number.

| Rule | Owner |
| --- | --- |
| `rustfmt` formatting | `rustfmt.toml`, `scripts/fmt-check.sh` |
| Type/borrow-check | `cargo check`, `scripts/type-check.sh` |
| unwrap/expect/panic/todo/unimplemented/print_*/`too_many_lines`/pedantic | `Cargo.toml [lints]` + `clippy.toml`, `scripts/lint-check.sh` |
| File size (300 code lines), function size (100 lines) | `guardrails.config.json` (`fileSize.max`, `functionSize.max`), the `file-size` guardrails check and `clippy::too_many_lines = "deny"` |
| No `mod.rs` barrels | `guardrails.config.json` (`barrelNames`), the `no-barrels` guardrails check |
| Folder tree shape (which subfolders each module may have) | `guardrails.config.json` (`src.nested`), the `folder-tree` guardrails check |
| `snake_case` filenames | `guardrails.config.json` (`filenameCase`), the `filename-case` guardrails check |
| Required per-folder `README.md` | `guardrails.config.json` (`src.requireReadme`, `src.nested "x/*"`), the `folder-readmes` guardrails check |
| No inline `#[cfg(test)] mod tests { ... }` | `scripts/guardrails/patterns/rust/no-inline-test-module.yml` |
| No direct `std::env::var` outside `config/`/`tests/` | `scripts/guardrails/patterns/rust/no-direct-env-var.yml` |
| No `unsafe` without a `// SAFETY:` comment | `scripts/guardrails/patterns/rust/no-unsafe-without-safety.yml` (project-local, D1) |
| No `#[allow(...)]` in production code | `scripts/guardrails/patterns/rust/no-allow-attribute.yml` (project-local, D6) |
| Committed secrets, shadow configs (`lefthook.yaml` vs `.yml`) | `guardrails.config.json` (`secrets`, `shadowConfigs`), the `secrets`/`shadow-configs` guardrails checks |
| Typos | `typos.toml`, `scripts/typos-check.sh` |
| `docs/cli-reference.md` drift | `scripts/cli-docs-check.sh` vs the real `--help` output |
| Shipped migration SQL is immutable | `scripts/migration-check.sh` (git-tag-dependent; compares each `src/store/sql/*.sql` against its first release tag) |
| Duplication ratchet | `scripts/dup-check.sh` against `dup-baseline.txt` (see `docs/dup-debt.md`) |

Additional gates wired into `just qa`: `scripts/deny-check.sh`
(`cargo deny check`), `scripts/dup-check.sh`, and `scripts/machete-check.sh`
(unused dependencies). `scripts/test-run.sh` runs the nextest suite. A task is
not "done" until `scripts/check-all.sh` exits 0.

## Distribution

- `curl … https://get.comemory.io/pkg/comemory/install | bash` — a 302 to the
  release installer below, served by the `comemory-prod` Cloudflare Worker
  (`apps/get` in the `CodaSignal/comemory.io` repo). The short URL is the stable
  one; the script behind it is regenerated by cargo-dist every release. The
  command with its hardening flags is spelled out in README § Install.
- `cargo install --path .` (build from a local checkout; not published to
  crates.io).
- `brew install Falconiere/tap/comemory` (Homebrew tap
  `Falconiere/homebrew-tap`, published by `cargo-dist`).
- Prebuilt tarballs for `aarch64-apple-darwin`,
  `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu` attached to
  [GitHub Releases](https://github.com/Falconiere/comemory/releases).

`cargo-dist` is configured in `[workspace.metadata.dist]` in `Cargo.toml`.
PRs get a dry-run plan; only version tags (e.g. `vX.Y.Z`) publish artifacts.

Releases are driven by the **release-plz** bot (`.github/workflows/release-plz.yml`,
config `release-plz.toml`): a push to `main` opens/updates a "release PR" that bumps
the version + rewrites `CHANGELOG.md` from conventional commits; merging it pushes the
`vX.Y.Z` tag, which fires `release.yml`. release-plz owns version + changelog + tag;
cargo-dist owns build + GitHub Release + Homebrew (`git_release_enable=false`,
`publish=false` — crates.io stays off). The bot is gated behind the
`RELEASE_PLZ_ENABLED` repo variable (a *variable*, not a secret — `vars` cannot
read secrets, so a secret of that name leaves both jobs silently skipping) and
mints a GitHub App installation token (`HOMEBREW_APP_ID` +
`HOMEBREW_APP_PRIVATE_KEY`, Contents + Pull requests read/write, the App
installed on this repo) so the pushed tag triggers downstream workflows. The
`just release` recipe remains a manual fallback. See `docs/release.md`.

## Claude Code Hooks

`.claude/settings.json` wires **only** the toolu-conventions guardrails
entries. comemory's own dispatcher layer (`.claude/hooks/`, adapted from
`qwick-business-app`) is **retired** — one enforcement surface, not two:

- **PostToolUse** (`Edit|Write` matcher) — `bash scripts/guardrails/run.sh --hook`,
  file-addressable structure checks on the just-edited path(s).
- **Stop** — `bash scripts/guardrails/run.sh --stop`, the full repo-mode
  guardrails sweep behind two early-outs.

The guardrails hook writes to stderr and exits **2** (Claude Code ignores a
`1` from a hook; only `2` surfaces on `PostToolUse` or blocks on `Stop`).
That contract is why it was never routed through the old dispatcher, which
swallowed stderr and always exited 0 — the layer would have been inert while
looking correctly wired. It is now the only entry, so the question is moot.

### What retiring the dispatcher gave up, and what covers it now

The dispatcher was a *tool-call interceptor*; guardrails is a *repo-state
checker*. They never overlapped, so this is a real reduction rather than a
consolidation — recorded here so nobody assumes guardrails absorbed it:

| Retired check | What covers it now |
| --- | --- |
| `npm`/`bun`/`pip`/`uv` blocked (Rust project) | Nothing. Convention only. |
| `rm -rf`, `git reset --hard`, `git checkout .`, `chmod -R 777` blocked | The harness's own confirmation on destructive commands. |
| `git push --force` blocked | Nothing — and note the old rule also caught `--force-with-lease`, the *safe* variant, so a rebased branch had no sanctioned way to publish. |
| `--no-verify` / `--no-gpg-sign` blocked | `lefthook.yml` still runs on commit/push, and CI re-runs `scripts/check-all.sh` regardless — a local bypass cannot land. |
| Direct `rustfmt` / `cargo clippy` blocked outside `scripts/` | Nothing. `scripts/fmt-check.sh` and `scripts/lint-check.sh` remain the canonical invocations. |
| `protected-files.sh` (build artifacts, `scripts/guardrails/**`) | Convention only — `scripts/guardrails/` is still copied verbatim from the kit and must not be hand-edited; `guardrails.config.json` and `scripts/guardrails/patterns/rust/` remain the sanctioned knobs. |
| `auto-format.sh` re-ran `rustfmt` on touched files | Deliberately gone. It invoked `rustfmt` without the project's edition, so it reordered imports into a form `scripts/fmt-check.sh` then rejected — it manufactured the drift it existed to prevent. |
| `gate-status.sh` session gate tracking | Nothing. `bash scripts/check-all.sh` on demand. |
| `session-end.sh` ran `fmt-check` + `typos-check` at Stop | The guardrails `--stop` sweep, plus lefthook's pre-commit `fmt`/`typos` jobs. |

The load-bearing gates were never in this layer: `lefthook.yml` runs `fmt`,
`guardrails` and `typos` pre-commit and `check_all` pre-push, and CI runs
`scripts/check-all.sh` on every PR. Those are unchanged.

User-facing docs live under `docs/`, organized in Diátaxis tiers and indexed by
`docs/README.md`: the `docs/getting-started.md` tutorial, the task-oriented
`docs/guides/` how-tos (byo-vectors, auto-reindex, ranking-and-eval, http-api,
prune-and-gc), the `docs/cli-reference.md` reference (every subcommand, flag, and
env var), and the `docs/architecture.md` explanation. The README is a front door
that links into them.

## Deviations from toolu-conventions

Every deviation below is deliberate and restated here per CORE's rule that
"deviating is allowed — documenting the deviation in the project's
`CLAUDE.md` is not optional." None weakens the kit's intent; several make the
local rule strictly stronger than the one it replaces.

- **D1 — `unsafe_code = "forbid"` is NOT set.** The crate has 7 FFI-necessary
  `unsafe` blocks and 3 `unsafe extern "C" fn` items, all in
  `src/store/tokenizer/ffi.rs` and `src/store/connection.rs`: registering a
  custom FTS5 tokenizer through `libsqlite3-sys`'s C ABI and registering
  `sqlite-vec` as a SQLite auto-extension have no safe wrapper in the
  ecosystem. `forbid` is a hard rustc error with no local override, so it
  cannot be applied. The key is omitted (rustc's default is `allow`) and
  replaced with a stronger, machine-enforced rule instead:
  `scripts/guardrails/patterns/rust/no-unsafe-without-safety.yml` fails the
  gate on any `unsafe` block or `unsafe fn` lacking a `// SAFETY:` line in the
  comment block directly above it — strictly stronger than the bash check it
  replaced, which never inspected an `unsafe fn` signature at all.
- **D2 — `fileSize.max` is 300, not the kit's 500.** comemory's module
  decomposition is designed around 300 code lines. CORE permits a stack to
  add rules but never relax one; 300 is stricter, so this is compliant.
- **D3 — `barrelNames: ["mod.rs"]`** (the kit ships `[]`, which leaves
  `no-barrels` inert for Rust). Declaring `mod.rs` a barrel name makes the
  `<dir>.rs` beside `<dir>/` layout permanent and machine-checked.
- **D4 — `src.nested` is extended** beyond the kit's `{"*": ["tests"]}` to
  allow `src/store/sql/`, `src/store/tokenizer/`, `src/store/migrate/`,
  `src/tui/view/`, and a universal `proptest-regressions` allowlist entry
  (proptest creates that directory itself on a failing property test; a gate
  that fails on a tool's own artifact is a gate people route around).
- **D5 — `src.requireReadme` is added** (the kit ships nothing). All 20 of
  comemory's grown module folders carry a `README.md`; the source material was
  already this file's Module Map, transformed rather than newly written.
- **D6 — two project-local ast-grep pattern rules**
  (`no-unsafe-without-safety.yml` for D1, `no-allow-attribute.yml` for Binding
  Rule 5's `#[allow]` ban) are **additions** to `scripts/guardrails/patterns/rust/`;
  no kit file is modified, so a future `cp -R` re-copy of the kit merges
  rather than clobbers them.
- **D7 — eight `clippy::pedantic` lints start at `allow` with a counted
  burn-down**, each line in `Cargo.toml [lints.clippy]` carrying the measured
  count at migration time. `-D warnings` with hundreds of pre-existing
  warnings is not a gate, it is a red build; adopting `pedantic` while
  explicitly listing what is not yet met is honest and burnable. This is a
  declared crate policy in `Cargo.toml`, not an `#[allow]` bypass in code —
  the ban on in-code `#[allow]` is retained and machine-enforced by D6. See
  `docs/lint-debt.md` for the full list and burn-down order.
- **D8 — extra quality gates are retained** beyond the kit's four-step
  command (`fmt && clippy && guardrails && test`): `typos-check`,
  `cli-docs-check`, `coverage-check`, `eval-check`, `dup-check`,
  `machete-check`, `deny-check`, and the mutation job. None has a kit
  equivalent; all guard comemory-specific failure modes. They layer around
  the guardrails step, not in place of it.
- **D9 — `tests/common/` stays at the crate root** and is bridged into the lib
  test crate via `src/test_common.rs`, so colocated unit tests and crate-root
  integration tests share one copy of every fixture (Binding Rule 1). Its
  `#![allow(dead_code)]` is the migration's one sanctioned exception,
  exempted by filename in `no-allow-attribute.yml`.
- **D10 — `extern crate self as comemory;` is added to `src/lib.rs`.** This
  lets every colocated test file keep `use comemory::...` imports identical
  to the crate-root suite's, so a test reads the same wherever it lives.
- **D11 — 92 of 291 test files remain crate-root integration tests**
  (54 CLI surfaces, 33 driving the real binary via `assert_cmd`, 4 owning an
  `insta` snapshot) — the kit's own "one file per public surface" category,
  not an exception, at a ratio high enough to declare. The ratio is a hard
  floor, not inertia: see "Why crate-root `tests/` cannot be deleted" under
  Testing for the `CARGO_BIN_EXE_comemory` constraint that pins it.
- **D12 — RESOLVED, no longer a deviation.** `.claude/settings.json` was
  briefly a *merge* of comemory's `pre-tools`/`post-tools`/`session-end`
  dispatchers and the kit's two guardrails entries. The dispatcher layer is
  now retired and the file wires the kit's entries alone, so this file matches
  the kit and there is nothing left to declare. See "Claude Code Hooks" above
  for what the dispatcher used to catch and what covers each item now — the
  reduction is real, and the load-bearing gates (`lefthook.yml`, CI's
  `scripts/check-all.sh`) were never in that layer.
- **D13 — `benches/` is untouched.** The kit's STRUCTURE.md is silent on
  benches; Criterion harnesses have their own cargo semantics and are neither
  `src/` nor `tests/`. They carry only the canonical test/bench lint header.
- **D14 — the `[workspace] members = ["."]` stanza stays**, required by
  `cargo-dist`. The guardrails module keys workspace mode on the presence of
  `guardrails.workspace.json`, not on Cargo metadata, so this repo takes the
  single-repo path regardless.
