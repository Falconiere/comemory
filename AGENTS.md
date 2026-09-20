# AGENTS.md

This is the canonical agent guidance for this repository. `CLAUDE.md` points here.

## Project Overview

`comemory` is a Rust CLI that fuses engram-style developer memory, grepai-style
semantic code search, and ast-grep AST patterns into a single binary, knit
together by a SQLite-backed store (memory + code rows + edges). It is a
**standalone agentic-RAG toolbox**, invoked from the command line, over its
loopback HTTP API (`comemory serve`), or as an MCP stdio server (`comemory
mcp`) that speaks the same command cores as those two — still no in-process
LLM. Everything runs locally: markdown is the source of truth and one SQLite
file (`comemory.db`) backs FTS5 + `sqlite-vec` + edges.

## Conventions that are easy to misread

- **`requireReadme` names folders, not files.** `src/<module>/README.md` is the
  per-file index for the files inside `src/<module>/`. A single-file module
  needs no README of its own; it is listed in its parent folder's README and
  documented with its `//!` module doc. `guardrails.config.json` lists exactly
  the folders that require a README.
- **A `POST` is not automatically mutating.** `RouteEntry::mutating` controls
  whether the read-only gate refuses a route that writes to the store.
  `POST /jobs/{id}/cancel` is deliberately `mutating: false`, so a runaway job
  can be stopped on a read-only server.

## Where to start reading

- `src/main.rs` / `src/lib.rs` — binary entry and library surface
- `src/cli.rs` / `src/cli/save.rs` — command dispatch and the save flow
- `src/domains/retrieval/pipeline.rs` — search orchestration
- `src/store.rs` — SQLite connection management
- `docs/architecture.md` — full architecture explanation

## Architecture

- **Source of truth:** markdown files with YAML frontmatter at
  `~/.comemory/memories/{id}-{slug}.md` (override with `COMEMORY_DATA_DIR`).
- **Single SQLite file:** `~/.comemory/comemory.db` with `memories`,
  `memory_fts` (FTS5), `memory_substring` (external-content trigram FTS5), `memory_vec` (`sqlite-vec` `vec0`), `code_symbols`,
  `code_fts`, `code_vec`, `edges`, `schema_meta`, plus stats / repo-marker
  tables and — when candidate capture is enabled — the three
  candidate-observation tables `candidate_query_observations`,
  `candidate_observations` and `candidate_judgments` (#209).
  `rusqlite 0.40` with `bundled` + `load_extension` features.
- **Declared schema (toolu-orm 0.7):** every table in `comemory.db` is a
  `#[table]` / `#[fts5_table]` / `#[vec0_table]` struct in
  `src/store/schema_*.rs`, assembled by `store::schema::registry()`.
  `just migration <name>` diffs the structs against
  `migrations/<newest>.snapshot.json` and writes the next
  `migrations/NNNN_<name>.sql` plus a SHA-256 journal entry;
  `just migration-journal` journals a hand-written file when a change is
  easier to write than to declare (data backfills, rebuilds). Runtime APPLY is
  unchanged: `store::migrate` runs every file with `execute_batch`, keyed by
  `schema_meta` markers; no `_migrations` table. See
  `docs/guides/schema-migrations.md`.
- **Runtime queries:** use the declared toolu-orm table builders and the private
  `store::orm` execution bridge, preserving native SQLite errors and caller
  transactions. Unsupported SQL stays inside `store/` and is tracked in
  `docs/guides/runtime-orm.md`; never substitute `INSERT OR REPLACE` for a
  row-preserving upsert. `stats_counts::Corpus` selects scoped counters without
  passing SQL predicates across the store boundary. Historical migrations and
  independent test fixtures remain SQL.
- **Edges:** flat `(src_kind, src_id, edge_kind, dst_kind, dst_id)` rows
  (plus an integer `weight`) in the `edges` table replace the v0.1 kuzu
  graph. v6 adds code-graph kinds: `co_changed` (mined from git history)
  and `imports` (per-language import resolution), feeding a materialized
  PageRank on `code_symbols.rank_score`. Recursive CTEs handle multi-hop
  traversal.
- **AST extraction:** `ast-grep-core 0.45` + `ast-grep-language 0.45` (rust,
  typescript, javascript, python, go only).
- **Vectors are BYO.** No in-process embedder. Callers pass vectors via
  `--vector` (CSV) or `--vector-stdin` (JSON `{"embedding":[..]}`). A sample
  Ollama wrapper ships in `scripts/comemory-embed.sh`.
- **Output:** TTY via `owo-colors`, JSON via `serde_json`. Exit codes follow
  `sysexits.h`.
- **No in-process LLM.** All ranking is deterministic (RRF fusion of FTS5 +
  `sqlite-vec`, a tiered lexical fallback ladder ending in mined learned
  expansions, edge walks).

Migration 19 adds graph/history/listing indexes and `memory_substring`. Its
triggers maintain the substring index for every memory write; listing retains
literal `LIKE` verification and a scan fallback for queries under three characters.
The external-content FTS5 table stores trigram postings and reads body text from
`memories.body`, without storing another body copy.

## Key Commands

```bash
cargo install --path .          # build + install the binary locally
comemory setup                  # detect + apply this machine's/repo's onboarding
comemory completions --install  # install + register bash/zsh/fish/powershell completions
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
comemory architecture scaffold     # model the repo's components from the code index (save|show|check|learn)
comemory eval                      # score retrieval (recall@k, MRR) vs golden set
comemory benchmark --set S.yaml    # domain-aware offline benchmark: pool recall vs recall@k/MRR/nDCG@k
comemory judge <observation-id>    # reviewed relevance verdicts against a captured candidate observation (or report it)
comemory eval --history            # past eval/tune/bandit runs, newest first
comemory mine --apply              # distill query reformulations into expansions
comemory tune --apply              # grid-search ranking knobs into config.toml
comemory consolidate               # advisory near-duplicate cluster report
comemory upgrade                   # move this binary to the newest release (--check, --version, --force)
comemory auth login|status|logout  # org-scoped cloud login; `login` also runs the first sync
comemory sync                      # manual push/pull against the bound organization (memories, then the code index)
comemory watch                     # follow the workspace channel; pull on every nudge
comemory capture session --path F  # redact + POST a session receipt (or --dry-run)
comemory capture sources           # show platform capture consent (CLI cannot grant)
comemory capture install-hook      # Claude Code SessionEnd → capture session --from-hook
comemory distill --session-id <id> --transcript <path>  # propose candidates from explicit saves
comemory mcp                       # stdio MCP adapter: the eleven-tool catalog over JSON-RPC
comemory recall-status             # tracked queries, verdicts, saves and pending recalls for a repo
just migration <name>              # struct diff → migrations/NNNN_<name>.sql (+ snapshot + journal)
just migration-journal <file>      # journal a hand-written migrations/NNNN_<name>.sql
just migration-adopt               # restate the newest snapshot from src/store/schema*.rs
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
   `src/domains/graph/neighbors.rs` — has no folder and therefore needs no README of
   its own; it is listed in its parent folder's README, and its `//!` doc is
   its documentation. `guardrails.config.json`'s `src.requireReadme` names
   the folders, and `scripts/guardrails/run.sh` is what enforces it.
8. **Docs in sync.** A change to a user-facing surface (CLI flags, public API,
   config, env vars) updates `README.md`, `AGENTS.md`, `docs/` and the module's
   own `src/<module>/README.md` in the same change.
   `scripts/cli-docs-check.sh` enforces the `docs/cli-reference.md` half
   mechanically. The generator discovers visible top-level commands from
   `--help`; `tests/cli_scenario_catalog.rs` independently checks its section
   inventory against clap, so regeneration cannot silently omit a command.
9. **Real data, no mock-data tests.** A test that only proves a mock returns
   what the mock was told to return is banned — it hides integration breakage.

10. **`rusqlite` is confined to the store module.** `src/store.rs` and
   `src/store/` are the only production code that may name the driver; every
   other module takes `store::Connection` / `store::Transaction` and calls a
   `store::<table>` helper. No `Statement`, `Rows` or `rusqlite::Error` may
   cross that boundary in a return type — a read returns owned `Vec<T>`,
   `Option<T>` or a scalar, so `QueryReturnedNoRows` never becomes a caller's
   control flow. `src/errors.rs`'s `Error::Sqlite(#[from] rusqlite::Error)` is
   the single permanent exception, kept deliberately: removing `#[from]` would
   force a `.map_err(...)` at every `?` inside `store/`. Enforced by
   `scripts/store-chokepoint-check.sh` against `store-leak-baseline.txt`, a
   two-sided ratchet — it fails both when the count rises above the baseline
   and when it drops below it without the baseline being lowered. The baseline
   is `0`; it must never go up.

   **The boundary is two-way (#177).** `store/` must not call a domain
   service, algorithm or derivation either: it persists what it is given. A
   domain resolves whatever a row needs BEFORE the write and passes it in as
   owned or borrowed row data (`store::CreatedWindow`, `store::MemoryLinks`),
   and the post-write refresh of derived state is the caller's step, never the
   writer's. A store file may name a domain **type** as a passive data input —
   `Frontmatter`, `Ref`, `References` — and only through the explicit
   `passive_store_models` allowlist in `scripts/architecture-policy.json`,
   whose targets must be types, never functions or modules. Enforced by
   `scripts/architecture-check.sh` (`store service dependency`); the
   `store_callbacks` allowlist that used to carry the exceptions is now
   empty.

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
| `cli/off_runtime.rs` | `off_runtime` — runs blocking platform I/O on a scoped thread. `main` is `#[tokio::main]`, so every subcommand body sits inside an async context, and `reqwest::blocking` panics on drop there (`Cannot drop a runtime in a context where blocking is not allowed`). `block_in_place` does not help — the runtime handle stays current, which is what reqwest objects to. Used by `cli::auth` and `cli::sync`; `serve` needs nothing (it already runs its command cores in `spawn_blocking`) |
| `cli/watch.rs` | `comemory watch` — the only long-lived CLI command: mints a 60-second ticket, holds the platform's workspace channel open (`tokio-tungstenite`, async on the existing runtime), and runs the same cursored `run_pull` on every `hello`/`change` frame. Frames are nudges: nothing is read out of one except that it arrived, so a missed frame costs latency and a duplicate costs an empty pull. Reconnects with jittered backoff (1s → 30s) |
| `cli/` | clap subcommand entry points + the top-level dispatcher in `cli.rs`, plus `output/` (the TTY/JSON writers, moved in from the top level by #178), `completion_script` (completion-script generation for both `comemory completions` and `GET /api/v1/completions` — the one sanctioned HTTP-to-CLI bridge: generating a completion script *is* clap work, so it stays beside the clap definition it reflects rather than being forced into a capability, and `serve::routes::meta` imports `cli::{Cli, completion_script}` for exactly that and the `GET /commands` inventory), `completion_install` (the idempotent per-user Bash/Zsh/Fish/PowerShell installer behind `comemory completions --install`; `install.sh` and the developer installer call it after placing the binary), and `pagination` (the flattened `--limit`/`--offset` clap struct). The shared date-flag layer moved to `utilities::when` with #166; `edges` is the fourth free-text surface (`comemory edges <query>` — lexical search over `edge_fts`, self-healing an empty index on first use); `graph` keeps only the clap surface and the `--format` renderers since #170 |
| `cli/setup/` | the terminal surface of `comemory setup`: `wizard` (the cliclack prompts — deliberately the thinnest file in the command, holding no decisions, so the one file a pty test would be needed for has nothing in it worth testing; cancellation arrives as `ErrorKind::Interrupted` and becomes `Ok(None)`, and an all-satisfied plan short-circuits rather than building the empty multiselect cliclack rejects) and `render` (the non-interactive summary, written into an `impl Write` so `tests/cli__setup.rs` snapshots it). `cli/setup.rs` owns the clap `Args`, the `Intent`/`Prompting` → `Mode` decision, and the exit-code mapping |
| `utilities/embed.rs` | shared embed-command shell-out (single-file module, no children) — runs `COMEMORY_EMBED_CMD` / `--embed-cmd` as `sh -c <cmd>`, feeds the query on stdin, parses `{"embedding":[..]}`. Consumed by `serve` (`POST /api/v1/doctor/reembed`, the server-side re-vectorizing job; `GET /health` reports `embed_cmd_configured`). `EMBED_TIMEOUT` bounds the stdout read rather than the whole execution, so a child that never drains a large stdin can block the caller on the write |
| `serve/` | loopback `/api/v1` HTTP server (`comemory serve`, API-only — the embedded web viewer was removed): axum `router` (mounts `routes::v1_router` behind the path-aware `guard` middleware — enveloped `401`/`403` JSON on `/api/v1/*`, plain text on any other `/api/*` path — and the 5 MiB `BODY_LIMIT`), `scope` (`RepoScope` — the per-request default `repo` filter: `X-Comemory-Repo` header first, the server's `--repo` second, never overriding an explicit parameter), `security` (session token generation/matching and the loopback Host guard; the canonicalize-and-contain checks `resolve_within` and `contain_abs` are transport-neutral and live in `utilities::path_containment`), `envelope` (the `{ok,data,meta}` / `{ok,error,meta}` `/api/v1` response envelope plus the one `Error → (StatusCode, code)` mapping table every HTTP error and every failed job's `{code, message}` derives from, with an optional structured `error.details`). `routes/` and `jobs/` are documented in their own rows below |
| `mcp/` | `comemory mcp` — the stdio MCP adapter, the third delivery surface beside `cli` and `serve`, sharing `utilities::blocking::run_blocking` and `utilities::error_code::classify` with `serve` rather than reimplementing either: `src/mcp.rs` (`McpOptions`, `serve` — open the store, build `McpState`, run the rmcp service over stdio until EOF), `catalog.rs` (`TOOLS` — the curated eleven-row `(name, command, mutating, description)` table `tests/mcp__parity.rs` walks against `Cli::command()`, ordered reads-then-writes: `find`, `search`, `search_code`, `context`, `show`, `list`, `edges`, `repos`, `recall_status`, `save`, `feedback`), `state.rs` (`McpState` — the per-session `Arc<Mutex<()>>` gate + `Paths` + `Config` + default repo + `read_only`; `exec.rs` owns each call's live connection), `server.rs` (`ComemoryServer` — `#[tool_handler] impl ServerHandler`, `read_router() + write_router()`, `get_info`'s five-line `instructions`), `tools_read.rs` / `tools_write.rs` (`read_router`/`write_router` — the nine read tools and the two write tools, each resolving scope then calling the matching `domains::<capability>::<cmd>::run` core, never reimplementing it), `exec.rs` (`run` + `Access` — clone state, `run_blocking`, lock the session and open/drop the live connection inside that closure, refuse an `Access::Write` outright under `--read-only`), `result.rs` (`into_tool_result` — `utilities::error_code::classify` into a tool-level `structured_error` for every class but `Internal`, a protocol `ErrorData` for `Internal`), `scope.rs` (`default_repo` — `--repo`, else the cwd's main-worktree label via `domains::code::git_utils::repo_label_at`, else `None`), `params.rs` (`FeedbackParams` — the one MCP-local parameter type: `confirmed_by_user` replaces the core's `source`, so an unstated verdict lands `implicit` and never enters the golden harvest). Every `repo` parameter left unset resolves to the session's default scope EXCEPT `repos`, which stays unscoped like `GET /api/v1/repos` since it is the discovery surface naming which labels exist; a `save` whose `repo` is still empty after that resolution is refused `repo_required` before the store is touched. A linked worktree is never a repository — scope is always the main worktree's basename. Owner `delivery::mcp` in `scripts/architecture-policy.json`: this folder never imports `cli` or `serve`, and no domain, `config` or `utilities` file may import it |
| `serve/routes/` | the versioned `/api/v1` REST surface: `routes.rs` aggregates every resource's `table_entries()` into one route table (method/path/CLI-command/`mutating` flag — the source of truth for the read-only gate, `GET /commands`, and `tests/api__parity.rs`) and owns the handler-layer helpers every resource shares — `query_response` (runs `domains::<capability>::<cmd>::run`, and the connection-mutex guard it takes, entirely inside `utilities::blocking::run_blocking`'s `spawn_blocking` closure, never across an `.await`), `respond`/`accepted` (envelope a result / a job-acceptance), `guard_mutating` (read-only-then-write-permit gate for a synchronous mutating route: `405 read_only`, else `503 busy` + `Retry-After` on permit contention), `guard_job` (read-only-only gate for a job-creating route — it always answers `202` immediately, permit contention only delays the job itself), `require_confirm` (the `confirmation_required` gate; its doc comment states the read-only-outranks-confirm ordering, AC-19), `track_for` (shared access-tracking suppression for `search`/`search-code`/`context`). Per-resource files: `memories/` (`memories.rs` — `GET /memories`, `GET /memories/{id}`; `search.rs` — `GET|POST /memories/search`, `GET|POST /context`; `write.rs` — `POST /memories`, `DELETE /memories/{id}?confirm`, `POST /feedback`), `code.rs` (`GET|POST /code/search`, `POST /code/ast` with pre-run containment, job-backed `POST /code/index` and `POST /code/ingest` under its own 64 MiB body-limit layer), `graph.rs` (`GET /graph`/`GET /edges`, reusing `domains::graph::query`'s `build_code_graph`/`build_graph_page` pair — no second query path), `sources.rs` (`GET /sources`, job-backed `POST /sources`, `DELETE /sources?target=&confirm=`), `learning.rs` (job-backed `POST /eval` read-class, `POST /tune`/`POST /bandit` confirm-gated only when `apply`, `golden` containment before every other check), `maint/` (`maint.rs` — `GET /doctor`, `GET /consolidate`; `prune.rs` — `GET|POST /prune`, `POST /gc`, plus `split_confirm` — the shared raw-body confirm-field extractor every confirm-gated route with a real `Request` type reuses; `admin.rs` — `POST /mine`, `POST /hooks/install`, job-backed `POST /rebuild` + the shared-connection swap), `meta.rs` (`GET /completions`, `GET /commands` — the clap-introspected route/command inventory), `stats.rs` (`GET /stats`), `repos.rs` (`GET /repos`), `find.rs` (`GET|POST /find` — its own resource because it is cross-domain, not a memories sub-resource), `hooks.rs` (`GET /hooks` read, `POST /hooks` per-hook toggle behind the read-only gate; NOT confirm-gated, since writing a hook file is idempotent and reversible), `jobs.rs` (`GET /jobs`, `GET /jobs/{id}`, `GET /jobs/{id}/events` SSE with `status`/`progress`/`log` events, `POST /jobs/{id}/cancel`). The console-api design (2026-09-01) added, as additional flat resource files: `overview.rs`, `search.rs` (the console view over `find` + suggest + per-hit feedback), `trash.rs`, `graph_nodes.rs`, `index_runs.rs`, `repos_admin.rs`, `learning_console.rs`, `config.rs`, `memory_stores.rs`, plus `memories/edit.rs` (PATCH/restore/refresh) and `maint/{doctor,gc}.rs` — see `src/serve/routes/README.md` for the per-file route list |
| `serve/jobs/` | the background job model for long-running commands (`index-code`, `ingest-code`, `index`, `rebuild`, `eval`, `tune`, `bandit`, `graph-recompute`, `reembed`, `gc`, `store-sync`): `registry` (`Arc<Mutex<HashMap<JobId, Job>>>` plus one retained `watch::Sender<JobStatus>` per job, so a late SSE subscriber's `borrow_and_update()` still replays a terminal status; a per-job `broadcast` log channel behind the SSE `log` event; a per-job cooperative cancel flag — `cancel(id)` marks a queued job `Cancelled` outright and asks a running one to stop at its next `ProgressSink::is_cancelled` boundary; `active_for(command, repo)` — the liveness check behind `409 index_running`; finished jobs beyond the 100 most recent are evicted on every insertion; job ids are 8 random `/dev/urandom` bytes, the same entropy source as the session token at a shorter width), `worker` (`spawn_job` — registers the job `Queued`, then on its own `tokio::spawn` task awaits the single write permit FIFO for `mutating` jobs only (a read-class job like `eval` never touches it), marks `Running`, runs the caller's closure — typically `domains::<capability>::<cmd>::run` over `Ctx::lazy`, the job's own connection — on `spawn_blocking`, and records the terminal `Done`/`Error` status). `JobView` also carries `progress: Option<Progress>` and a bounded 20-line `log_tail`, surfaced as a SECOND SSE event type (`event: progress`) — the `status` event payload itself is deliberately unchanged — `JobStatus` gained only the terminal `Cancelled` variant, so an existing client's `queued`/`running`/`done`/`error` events stay byte-identical. `events` holds the three SSE payload types. Lifecycle is `Queued → Running → Done \| Error \| Cancelled`, not persisted: a server restart forgets every unfinished job |
| `store/` | **the SQLite chokepoint: the only production module that imports `rusqlite`** (`src/errors.rs` excepted, see Binding Rule 10), calling no domain service in return (Binding Rule 10's second half), and the single home of every SQL string in the crate (migration DDL lives beside it in the crate-root `migrations/`). `src/store.rs` re-exports `Connection` and `Transaction` so no other module names a driver type. Flat files, one per table or concern; families that outgrow 300 lines become flat siblings with a shared prefix (`rebuild_copy_*`, `code_graph_*`, `edges`/`edges_retrieval`) because guardrails' `src.nested` allows no new subfolder under `store/`. Core: `connection` (rusqlite open + PRAGMAs + `sqlite-vec` loader), `schema`, `migrate` (versioned + idempotent, applying the `MIGRATIONS` slice declared in `migrate/list.rs`; DDL text in the crate-root `migrations/`, `include_str!`-baked; `schema` + `schema_*` (the declared toolu-orm schema — `registry()`, `DECLARED_TABLES`, and the `#[table]` structs) + `schema_journal` (the `just migration-journal` / `migration-adopt` operations, tested against the shipped journal); `migrate/preflight` + `migrate/backup` are the forward-compat guard and pre-upgrade `VACUUM INTO` snapshot run from `connection::open` before the chain — see `docs/guides/upgrading.md`), `vector` (`vec0` insert/KNN with dim guard), `fts` (FTS5 helpers, code leg), `fts_memory` (the memory FTS ladder behind one `run_memory_match` choke-point — every tier inherits the same filters), `CreatedWindow` and `MemoryLinks` in `store.rs` (the borrow-only row inputs a caller fills in: the `{since, cutoff}` window each SQL predicate takes, compared via `datetime()`, and the `{files, symbols, documents}` reference targets `memory_row::insert` writes — the first keeps `store/` free of `retrieval/` types, the second free of `domains::graph` derivation), `embed` (`to_vec_blob`, dim helpers), `edge_fts` (FTS5 triplet index over `edges` — per-kind `src —rel→ dst` rendering, wholesale refresh-materialize in one tx, `needs_refresh` for the upgrade self-heal, and the two-tier strict→word-OR ladder behind `comemory edges`), `memory_meta` (`fetch_meta` — batched per-memory metadata: path/repo/kind/tags/references backing the enriched `search --json` rows), `memory_row`/`code_row` (the per-table mirror-row upserts; `memory_row::insert` takes its reference targets as `MemoryLinks` and is called only by `domains::memories::mirror`), `memory_list` (paginated memory listing, `--sort created|quality|accessed`), `eval_runs` / `gc_runs` / `index_runs` (the v14/v15 run-history writers + readers; all three are in `rebuild`'s `COPIED_TABLES` — history is not reconstructable from markdown), `candidate_observations` / `candidate_judgments` (the v20 candidate observation contract: the one-transaction write, the read back, the purge's redact-the-passage-keep-the-row rule and the retention sweep that never evicts a judged observation; also in `COPIED_TABLES`, and never synced), `candidate_dataset` (the bulk windowed read behind `comemory export-dataset`: all three of those tables in ONE read transaction, and deliberately never selecting `locator_json`, so a display field cannot reach a training record), `repo_drop` (`DELETE /api/v1/repos/{name}`: drop every code-index row and file edge for one repo label in one transaction, memories kept; the post-commit derived refresh is `domains::code::repo_admin::disconnect`'s), `random_id` (the shared random-hex id helper, moved out of `serve::security` so non-HTTP callers can use it), `code_ref` (the version-anchor side table for explicit code references), `documents`/`document_fts` (the document/chunk mirror + its BM25 leg), `sources` (the SQLite mirror of `source::registry`), `simhash_scan` (bulk fingerprint scan shared by save + consolidate), `tokenizer/` (custom FTS5 identifier tokenizer: camelCase/snake_case split + FFI registration), `busy` (`is_locked` — answers "is this SQLite busy/locked?" from the crate's own `Error`, so `serve::envelope` maps 503 without inspecting driver variants), plus the per-table modules the 2026-09 chokepoint refactor moved in from `api/`, `cli/`, `graph/`, `stats/`, `retrieval/`, the retention rules and `eval/`: `edges`/`edges_retrieval`/`code_graph_nodes`/`code_graph_edges`, `retrieval_log`, `feedback`/`code_feedback`, `bandit_arms`, `query_expansions`, `indexed_files`, `index_failures`, `repo_marker`/`repo_marker_roots`, `prune_signals`/`prune_apply`, `gc_learning`, `doctor_probes`, `repos_inventory`, `stats_counts`, `trash_list`, `code_signals`, `schema_meta`, `sync_log`/`sync_state`/`sync_binding`, `code_sync` (the projection reads and the `code_sync:<repo>` cursor behind the code-index push), and `rebuild_copy*` (the ATTACH→copy→DETACH preservation unit, which owns its own lifecycle so no caller can leave a database attached) |
| `domains/retrieval/unified/` | `comemory find`'s core (`unified::run_legs` is `find` minus fusion and pagination, returning each leg's own rows with their passage text and version anchors intact — what the offline benchmark reads so it cannot drift from the shipped ranking): the three legs (`router`, `code_route`, `doc_route`) run unchanged and their *reranked* orders fuse via the pre-existing `fuse::rrf_multi_weighted`, memory and code at weight 1.0 and documents at `retrieval.document_leg_weight` (declared and validated since the document domain landed; read by nothing until this module). One shared `pipeline::pool_size` across every leg and ONE `pipeline::paginate` over the fused list — RRF is prefix-stable, so divergent per-leg pools would let a deeper page reorder a shallower one. `fuse_domains` owns the weighted fusion and `UnifiedHit`/`HitParts`, the untagged enum carrying each domain's own `score_parts` verbatim |
| `domains/retrieval/` | the retrieval capability (#171): `router` (candidates + 4-tier lexical ladder: strict → word-OR → subtoken-OR → tier-4 learned expansion from mined `query_expansions`), `doc_route` (the document-search leg: BM25 over `document_fts`, chunk→parent coalesce), `scope` (`TimeScope` — the `--since`/`--until`/`--as-of` created-date window plus its as-of supersede semantics — and `Filters`, the `{repo, kind, scope}` bundle every leg narrows candidates by), `score` (ACT-R/Beta scoring primitives), `rerank` (five multiplicative priors over the max-normalized relevance: activation × feedback × quality × supersede × rank, the last being the pool-median-relative `memories.rank_score` boost), `diversify` (SimHash near-dup collapse + MMR), `pipeline` (orchestration + access tracking), `fuse` (RRF: pairwise `rrf_k` + N-ary `rrf_multi`), `graph_route` (graph-expansion leg — one undirected recursive-CTE walk over `edges` from the provisional top hits, fused in as a third RRF list; `graph_hops = 0` or an empty expansion returns the provisional ranking untouched), `bundle` (context lookup with graph-prior-ranked code refs), `code_route` (code candidates: BM25 + thresholded ANN + RRF, chunk→parent coalesce), `code_rerank` (four-prior code rerank), `code_prior` (PageRank / recency / working-set affinity / feedback priors), `code_search` (`search_code_hits` — the shared code-search entry point used by both the `search-code` CLI and `GET|POST /api/v1/code/search`), `unified/` (the `comemory find` core — see its own row), `code_ref_collect`/`code_ref_fetch`/`code_ref_status` (pinned code-reference freshness: collect a memory's walked ref edges, fetch per-repo current state through `utilities::repo_root`, classify `fresh|stale|ghost|unpinned|unknown`), and — arriving with the capability — the six cores both adapters call (`search`, `search_code`, `context`, `find`, `suggest`, and the console-only `config_retrieval`), the result models and the `--json`/`/api/v1` envelopes both transports serialize (`search_result`, `code_search_result`, `context_result`, plus `scope`'s `ScopeEcho` and its transport-neutral `resolve_domains` `--only` policy), and `explain` (the explain strip: a hit's `score_parts` as `{name, value, share, note}` rows, `share` being the log-magnitude partition of the multiplicative priors). Clap flags, TTY colouring and `--json` emission stay in `cli/` (the writers in `cli/output/`) |
| `config/` | layered config (defaults → `file` → `env`) and `paths::Paths` (data-dir layout), the `learning` section (`[tune]` grids, `[reinforce]`, `[bandit]`), the `retrieval` section (the `[retrieval]` knobs), `validate` (the shared invariant pass run after every layer is applied), and `patch` (`patch_config_file` — the one read-patch-atomically-write primitive over `config.toml`, shared by `tune --apply`, the `hooks` reinforce toggle, and the console's `PUT /config/retrieval` / `PUT /gc/policy` / `PATCH /memory-stores/{id}`; every writer validates the would-be `Config` in memory before touching the file) |
| `cli/output/` | TTY (`owo-colors`) and JSON (`serde_json`) writers, shared between subcommands. CLI presentation only, and since #178 that is literally true — `serve` imports nothing from here: the `/api/v1/edges` body is built by `domains::graph::edges_result::envelope` and the `comemory serve` startup banner is written by `cli::serve` from the `serve::Ready` handover. Each retrieval command's `--json` envelope and rows are what both transports serialize, so they live in `domains::retrieval::{search_result, code_search_result, context_result, scope}` and the explain strip in `domains::retrieval::explain`; `cli::output::{search, search_code, context}` keep `emit`/`write_tty` (#171), and `cli::output::edges` keeps only the TTY writer (#178). `comemory::output` still resolves for library consumers through the crate-root alias in `lib.rs` |
| `utilities/` | transport-neutral shared primitives, usable by every domain and by both delivery adapters, one concern per file: `blocking` (the `spawn_blocking` bridge both delivery adapters share: run a closure on the blocking-thread-pool, flattening a task panic into `crate::Error`, so a connection lock never crosses an `.await`), `context` (the `Ctx` every command core takes: `Paths` + `Config` with a connection that is either `Borrowed` — the CLI's own, or the server's shared per-request one — or `Lazy`, opened on the first `Ctx::conn()` call, which is a job worker's own dedicated connection; conn-free commands like `install`, `doctor`, `rebuild`, `ast`, `install-hooks` and `completions` never open one at all. cwd-dependent middles — `save --ref-*` anchoring, the code rerank's working-set prior — resolve against the *calling process's* cwd, which over HTTP is the server's and not the client's: documented behavior rather than a bug), `pagination` (`Page<T>`, `PageWindow`, `PageMeta`, `page_window`, `page_meta`), `id_list` (CSV split/dedupe + the two id-list parsers), `when` (`--since`/`--until`/`--as-of` values), `ref_args` (`--ref-file`/`--ref-symbol` → `References`), `embedding_input` (pure `--vector` CSV / JSON payload decoding) and `vector_stdin` (the process-stdin acquisition split out of it), `error_code` (`classify`: the transport-neutral `Error → (code, Class)` mapping `serve::envelope::status_and_code` and `mcp::result::into_tool_result` both derive their own status/error shape from, an exhaustive match with no wildcard arm), `embed`, `fetch`, `http_error`, `simhash`, `digest` (SHA-256 hex + the lowercase-hex shape check), `file_lock`, `path_containment`, `progress` (`ProgressSink`), `query_id`, `repo_root` (the one repository resolver — `file:<repo>:<path>` id to a contained absolute path, its `parse_id` decoder, and the `--root` override map — shared by `cli`, `serve`, retrieval freshness and reference refresh since #167), `telemetry` (the persisted `retrieval_log.source` / `feedback_events.{target_kind,provenance}` vocabularies), and `dated_id` (the `<prefix>-<yyyymmdd>-<8hex>` shape both `query_id`'s `q-` ids and `observation_capture`'s `o-` ids are minted and validated through). Nothing here may import `cli` (the `cli::output` writers included) or `serve`, and a reference into a capability is a `shared layer dependency` failure unless `scripts/architecture-policy.json` declares it: `id_list` validates a memory id through `domains::memories::id`, and `ref_args` resolves blob OIDs through `domains::code::git_utils` and produces `domains::memories::{Ref, References}` |
| `domains/` | business capabilities, one folder each, declared from `src/domains.rs`. A capability owns its models, algorithms and command cores; it may depend on `store`, `config`, `errors`, `prelude`, `utilities`, and — only through the directed table in `scripts/architecture-policy.json` — another capability. It may never import `cli` (including the `cli::output` writers) or `serve`, and may never exist as an empty scaffold (the architecture gate fails one). Every core's `Request` derives `#[serde(deny_unknown_fields)]`, enforced by `tests/api__parity.rs`'s clap-introspection walk, so a typo in a flag or a JSON field is rejected rather than silently ignored on either transport |
| `domains/architecture/` | the architecture capability: the component-level model of a repository — `model` (the versioned `{groups, components, edges}` value plus the 200-component / 2000-edge / 32 KiB / 280-character ceilings), `cluster` (directory-prefix keys, renderer-safe ids, README-seeded summaries), `scaffold` (the deterministic model built from `graph::query::build_code_graph` and `store::indexed_files`), `validate` (every save-time rule, checked against the indexed paths so a component cannot claim a file the index has never seen), `save` (validation then `memories::save::run` with `kind: note`, `tag: architecture` and a `supersedes` link to the previous model; `store::memory_row` writes the row, this capability never issues SQL), `current` (the newest live tagged memory and the fenced JSON inside it), `check` (stale members, unmapped clusters and missing mined edges; report-only, like `doctor`), `mermaid` (deterministic `flowchart` source), and the `learn` trio `prompt` / `learn` / `extract` — a prompt file, one `sh -c` spawn of the command the CALLER passed under a `tokio::time::timeout`, and the model dug out of its stdout. comemory ships no agent command and reads none from disk. CLI-only (`serve::routes::meta::CLI_ONLY`); clap flags, the `--format` switch and every rendered line stay in `cli/architecture.rs` and `cli/output/architecture.rs` |
| `domains/capture/` | the capture capability (#174): client-side coding-session capture and explicit-save distillation against the platform — `claude_code` (Claude Code JSONL → session metadata and Bash `tool_use` lines), `receipt` (redact + SHA-256 digest → the Slice 3 wire receipt), `redact` + its own `rules.toml` (the client rule set and the versioned attestation every payload carries — deliberately a DIFFERENT rule set from `domains/sync/rules.toml`, not a duplicate to merge), `explicit_save` (recover `comemory save` claims, `claude-code-explicit-save`), `candidates` (batch types + `POST /v1/sessions/{id}/candidates`), `client` (`POST /v1/sessions`, `GET /v1/capture/sources`), `run` and `distill` (the two use cases), and `hook` — the `SessionEnd` contract on both ends: the command installed into the tool's settings AND the payload that command is handed back on stdin. Credentials and the API base come from `domains::sync` by concrete path; installing the plugin BUNDLE is integrations' (#175). CLI-only (`serve::routes::meta::CLI_ONLY`), entered from `cli/capture.rs` and `cli/distill.rs`, which keep clap, stdin, rendering and exit codes |
| `domains/code/` | the code capability (#167): `ast` + `ast/` (symbol extraction and AST pattern search via ast-grep, cAST chunking, the compiled-pattern cache), `git_utils` (blob OID / HEAD / branch / remote lookups, hook installation helpers, and `repo_label` — the main-worktree basename rule), `pattern_search` (`comemory ast`), `index_code` + `index_code/walk` and `ingest_code` (the DB-write index paths), `index_runs` (history), `repos` + `repos/git_state` and `repo_admin` (the repository inventory and its admin routes), `hooks` / `install_hooks` (the git reindex hooks, resolved through the git common directory), and `reindex_policy` (the staleness/debounce decision behind a lazy reindex — the detached `index-code` launch stays in `cli::lazy_reindex`) |
| `domains/documents/` | the document capability (#168): `document` + `document/` — pure, in-process extraction (TXT/Markdown/HTML/CSV) and size-bounded chunking, independent of the store: `extract`/`html`/`delimited` (format-specific extractors), `chunk` (the shared paragraph-boundary splitter), `fingerprint` (size+mtime skip check, SHA-256 identity), `writer` (the per-file index writer: fingerprint skip → extract → one-transaction row replacement, which also calls `graph::doc_link` — the single membership/reference derivation path); `source` + `source/` — the durable `sources.toml` registry: `registry` (load/save, overlap validation, atomic durability; it takes the shared `utilities::file_lock` guard over concurrent read-modify-write), `discover` (the boundary/ignore-rule walk over a registered root), `classify` (extension allowlist + binary sniff), `mirror` (reconciles the TOML registry into SQLite's `source_roots`); and the three command cores `index`, `sources` (whose `reconcile:false` mode is the read-only probe `domains::integrations::setup::detect` uses) and `unindex`. All SQL stays in `store::{sources,documents,document_fts}` |
| `domains/graph/` | the graph capability (#170): graph **algorithms** over the `edges` relation — the table's CRUD and every recursive-CTE walk live in `store::edges` / `store::edges_retrieval`, which these call. `cross_link` reference extraction (extraction only — `domains::memories::mirror` calls it and `store::memory_row` writes the edges, #177), `cochange` (git-history co-change mining), `imports` (per-language import edges), `pagerank` (deterministic weighted PageRank), `materialize` (writes `rank_score` onto `code_symbols`), `memory_rank` (the same PageRank over the memory graph — direct memory→memory relations plus in-memory co-citation edges, hub rels excluded — written onto `memories.rank_score`), `coactivate` (commit co-activation reward: a commit touching a memory's referenced files reinforces it), `doc_link` (deterministic `member_of_source`/`references_document` link deriver; the document-index seam `derive_after_document` writes its own edges, while the memory-save seam `resolve_memory_documents` returns the resolved ids for `domains::memories::mirror` to pass to the store), `search_edit` (search→edit lookback feeding `auto_search_edit` provenance), `derived` (`refresh_derived_best_effort` — the single post-write pass that refreshes *both* derived artifacts, `memories.rank_score` and the `edge_fts` triplet index, independently best-effort; every caller is a domain core running AFTER its own transaction commits — `memories::{save,delete,update}`, `maintenance::{rebuild,gc}`, `code::{index_code,repo_admin::disconnect}`, `sync::exchange::{import_write,code_import}` and `graph_recompute`), `neighbors` (the one-hop undirected `imports`/`co_changed` file neighborhood query shared by `retrieval::bundle` and `GET /api/v1/graph/nodes/{id}/neighbors`), plus the model and query assembly both transports share — `code_graph` (`Node`/`Edge`/`CodeGraph`/`GraphPage`), `query` (the `Rel` relation vocabulary and the `build_code_graph`/`build_graph_page` edge-window builders) and `nodes` (the windowed-edge endpoint dedup and `build_graph`) — and the four command cores `view`, `graph_nodes`, `graph_recompute` and `edges` |
| `domains/integrations/` | the integrations capability (#175): `install` (+ `install/bundle`) extracts the embedded `integrations/agent/**` assets into `<data_dir>/integrations/<version>/` as a local marketplace — staging directory then rename, refusing a user-edited bundle and a symlinked destination or catalog — probes the host CLI and the `bash`/`git`/`jq`/`python3` hook dependencies before writing anything, and registers the plugin with the host's own manager. It is connection-free, so installing never creates a database, and the `.installed-<host>` marker is per version AND per host because the extracted bundle is one shared tree: its existence alone would wrongly report every host as installed. Host names are validated strings, never clap enums. `install` also writes `<bundle>/plugins/comemory/.mcp.json` on every run (`bundle::write_mcp_manifest`) — the plugin-root manifest both Claude Code and Codex read to spawn `comemory mcp`, carrying the installing binary's absolute path, written temp-then-rename and deliberately outside the embedded-bytes equality check so moving the binary and reinstalling refreshes the path without a `bundle differs` refusal; `--dry-run` reports the path and writes nothing. `setup` (+ `setup/`) is the onboarding composition over seven stable step ids — `data-dir`, `agent-host`, `git-hooks`, `index-code`, `index-docs`, `reinforce`, `cloud-auth`, validated before anything is probed, `--skip` beating `--only`: `detect` is offline and read-only (it consults `domains::maintenance::doctor` only once a `comemory.db` exists, lists sources with `reconcile: false`, and reads `domains::sync::auth_file` rather than asking the platform), `plan` is pure, and `apply` is the only phase that writes, recording a step's error as its own `Failed` state so one failure cannot hide the others. `cloud-auth` and `index-docs` are report-only — setup points at `comemory auth login` / `comemory index <path>` instead of signing in or indexing an arbitrary root. Both cores are CLI-only, in `serve::routes::meta::CLI_ONLY` and the `tests/api__parity.rs` exception list; clap shapes and host aliases, terminal detection, the wizard, the rendered summary and the exit-code mapping stay in `cli` |
| `domains/learning/` | the learning capability (#173): feedback, evaluation and the searches over the ranking blend. The `feedback` core commits memory and code verdicts in one immediate transaction; a failure rolls back both. `feedback_tracking` (per-memory `used`/`irrelevant` counters, the routes' `explicit|implicit` `Source` word and its one mapping onto the stored `manual`/`implicit` provenance, and `record_implicit_used` — deliberately taking a bare `&Connection` so `domains::graph::materialize` mints the co-activation reward inside the transaction that also advances its cursor), `code_feedback` (the same for symbols, keyed by the stable `(repo, path, symbol)` identity with the chunk-to-parent walk, since re-indexing recycles rowids), `telemetry` (`StatsDb`, the shared `comemory.db` handle those two borrow their transactions from), `evaluation` + `evaluation/` (`golden` YAML sets plus the `manual`-only feedback harvest, `metrics` recall@k/MRR/bootstrap CI, `runner` over the real pipeline with tracking off, `mine` reformulation mining into `query_expansions`, `tune`/`tune_sample` deterministic or seeded-sampled grid search, `bandit`/`bandit_rng` the eval-gated Thompson bandit on a dependency-free SplitMix64 + Beta/Gamma sampler), `recall_status` (read-only, no confirm gate: tracked queries, verdicts, saves and the still-pending recalls for a repo + lower time bound, shared by `comemory recall-status`, `GET /api/v1/learning/recall-status` and the MCP `recall_status` tool), the **domain-aware offline benchmark** added by #208 (`candidate_identity` + `candidate_observation` — the candidate observation contract #209 persists and #210 exports, specified in `docs/designs/2026-09-18-domain-aware-retrieval-benchmark.md`; `judgment` reviewed relevance targets; `benchmark_set` the versioned reviewed set with its pinned `ranking` knobs and pre-declared budgets; `candidate_facts` + `benchmark_observe` the identity/text/filter capture; `benchmark_runner` over `retrieval::unified::run_legs`, which has no telemetry path at all; `benchmark_metrics` candidate-pool recall apart from recall@k/MRR/nDCG@k with a paired bootstrap; `benchmark_arm` + `benchmark_arm_report` + `benchmark_report` the arms, verdicts and replayable artifact; `run_environment` the hardware, memory and pinned corpus/index snapshot), the **persisted** half of that contract added by #209 (`observation_capture` — opt-in, bounded, best-effort capture of a real `comemory find` pool, armed only when the operator opted in AND the run may write telemetry; `judge` — typed verdicts resolved against a captured observation, refusing a candidate-pool recall miss, a stale content version and an unresolvable candidate, all-or-nothing, and always `manual` because a typed verdict is a human one), the **reviewed dataset export** added by #210 (`dataset_export` — the command core, which validates before the database opens, reads one consistent snapshot and writes the JSONL files and manifest; `evaluation::dataset_{record,rows,select,dedup,split,build,files,manifest}` — the record shape and its `RECORD_VERSION`, the fixed pipeline order, the named drops and their counters, the grouped seeded split over the query/content graph, the bucketing that puts splits before negative selection, the owned-name sweep that makes the withheld holdout real, and the manifest with its `snapshot_digest` / `dataset_id`; specified in `docs/designs/2026-09-18-reviewed-dataset-export.md`), and the cores every adapter calls — `feedback`, `eval`, `mine`, `tune`, `bandit`, the CLI-only `benchmark`, `judge` and `export-dataset`, plus the console-only `console` (`GET /learning/*`) and `learning_proposals`. All SQL is `store::{feedback,code_feedback,eval_runs,query_expansions,bandit_arms,retrieval_log}`; the persisted vocabularies and the query-id contract are `utilities::{telemetry,query_id}` (#166), which retrieval and graph consume directly |
| `domains/maintenance/` | the maintenance capability (#176): operational corpus maintenance, one named file per operation — never a shared `operations.rs`. Health: `doctor` + `doctor/` (`checks`, one function per health probe; `backup`, the newest `comemory.db.pre-v{N}.bak` snapshot; `system`, the facts half behind `GET /doctor/system`). It probes without ever CREATING the database and falls back to a read-only open when the schema is newer than this binary — the transport-neutral read `domains::integrations::setup::detect` consults once a `comemory.db` exists (declared in `setup_runtime_dependencies`). Retention: `retention` + `retention/` (`low_value` signal/superseded rules, `orphans`, `stale_code` ghost `references_symbol` anchors) is read-only and side-effect free by construction, with the command core `prune` owning the confirmed `--apply`; `gc` reaps aged `.trash/` files AND purges their mirror rows (`store::memory_purge`) and learning telemetry (`store::gc_learning`) in the same sweep; `gc_policy` holds the retention windows. Duplicates: `consolidation` + `consolidation/` (`cluster` transitive union-find over stored SimHashes within a radius, `keeper` ordering and in-cluster supersede resolution) behind the read-only `consolidate` core — the merge stays a human `save --supersedes`. Repair: `rebuild` + `rebuild/copy` (the `COPIED_TABLES`/`RECONSTRUCTABLE_TABLES` allowlist pair; the ATTACH copy SQL itself is `store::rebuild_copy*`) snapshots the still-live DB to `comemory.db.pre-rebuild.bak`, then swaps atomically, with `serve::routes::maint::admin` refreshing the shared connection at the HTTP seam; `reembed` re-vectorizes through the embed command with a dimension guard and the shared progress/cancel sink. Dashboards: `stats` and `overview` are read-only, must-not-create-the-db, and compose `domains::code::repos` / `domains::memories::list` rather than restating their policy. The two owned report models are siblings of their algorithms — `retention_report`, `consolidation_report` — because the core, the CLI renderer and the HTTP envelope all build the same value. Upgrade: `upgrade` + `upgrade/` (`version` parse + ordering, `channel` Homebrew / `cargo install` / standalone detection from the resolved `current_exe`, `release` the `<releases>/latest` redirect → tag and one-asset `download` via `utilities::fetch` curl/wget — no HTTP client in the crate, `COMEMORY_RELEASES_URL` is the test hook, `installer` runs the release's own `install.sh` pinned `--version --dir --no-modify-path [--quiet]` or `brew upgrade` and reads `installed_version` back). `upgrade` is CLI-only: in `serve::routes::meta::CLI_ONLY` beside `serve` / `auth`, since a server must never replace its own binary on request |
| `domains/memories/` | the memory capability (#169): the markdown record (`frontmatter`, `id`, `slug`, `references`, `prior`) and its atomic `store`, plus the eight cores both adapters call — `save` (the content-addressed replay contract, validation before every effect, the near-dup advisory), `delete` (with the `soft_delete` / `mirror_soft_delete` helpers `prune` and the sync import reuse), `list`, `show`, `update` (+ `mirror_record`, the one re-mirror path), `restore`, `trash`, `refresh_refs`, and `nav` (a memory's title and markdown path), plus `mirror` — the single seam every writer here, in `maintenance::rebuild` and in the sync import goes through, which derives a body's graph links and hands them to `store::memory_row` as row data (#177). SQLite stays in `store`; clap flags, process I/O and the inline cloud push stay in `cli` |
| `domains/sync/` | the sync capability (#172): the org-scoped credential and the login that mints it — `auth_file` (auth.json schema v2; v1 refused; `load_usable` is the read-only probe `domains::integrations::setup::detect` uses), `cloud` + `cloud/` (`api_url` — `--api-url` > `COMEMORY_API` > `https://api.comemory.io` — and `device`, the RFC 8628 code/token poll plus `POST /v1/device/mint-org-key`; the device-code grant is only the approval channel, so the workspace travels *in the key* and a mint missing `organizationId`/`workspaceId` is refused rather than written), and `login` (the three `comemory auth` sequences with the progress writer injected and nothing rendered); the wire protocol's two halves — `client` (reqwest+rustls; Authorization only, plus `ws_ticket`/`channel_url` for the workspace channel), `client_code`, and `exchange` + `exchange/` (the *server* half the `serve` routes call: the wire models plus the `changes` / `manifest` / `import` / code-import cores); the directions of travel — `push`/`pull`/`verify`, `initial` (exhaustive pull-then-push at login, then the code push), `manual` (what one `comemory sync` run does), `push_on_save` (the inline drain after a local write, called from the CLI seam only so an HTTP write starts no outward sync), and `code` + `code_plan` + `client_code` (the code-index push: every indexed repo's snippet-free projection — paths, blob OIDs, symbol names and line ranges, `imports`/`co_changed` edges — diffed by blob OID against the workspace's `GET /v1/sync/code/manifest` and posted to `POST /v1/sync/code/import`); the filters and the resident loop — `redact`/`rules.toml` (the curated secret scan before a push is enqueued), `skip_repos` (the ONE client-side filter left — an empty `repo` label no longer withholds anything, since the label comes from the cwd's git repo and made sync eligibility depend on which directory `save` ran in), `watch` (hold the workspace channel and pull on every nudge, with full-jitter reconnect), and `daemon` + `daemon_unit` + `daemon_templates` (user LaunchAgent `io.comemory.sync` / systemd `--user` `comemory-sync.service`, OPT-IN via `auth login --daemon`); and `memory_store` + `memory_store/` — a different kind of sync: Git synchronization of the data directory's own work tree, with its own `store-sync` job and log contract. Clap flags, prompts and every rendered line stay in `cli`; all SQL stays in `store::{sync_log,sync_state,sync_binding,code_sync}` |
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
| `COMEMORY_LEARNING_RETENTION_DAYS` | `comemory gc` retention window (days) for raw `retrieval_log` + `feedback_events` rows, and for UNJUDGED captured candidate observations; aggregated `feedback` counters, mined `query_expansions` and any observation carrying a reviewed judgment never expire | `90` |
| `COMEMORY_OBSERVATIONS_ENABLED` | Opt in to candidate observation capture: `comemory find` records its whole candidate pool — each candidate's stable identity, content version and bounded passage — and prints the `o-<yyyymmdd>-<8hex>` handle `comemory judge` takes. Off by default, because capture stores passage snapshots. Capture additionally requires a run that may write telemetry, so a read-only `comemory serve` and `COMEMORY_DISABLE_ACCESS_TRACKING` both suppress it, and a capture that fails never fails the search (`[observations] enabled`) | `false` |
| `COMEMORY_OBSERVATIONS_MAX_TEXT_BYTES` | Per-candidate passage bound in bytes; the digest always covers the FULL passage before bounding. Validated `> 0` (`[observations] max_text_bytes`) | `4096` |
| `COMEMORY_OBSERVATIONS_MAX_CANDIDATES` | Ceiling on candidates persisted per captured query; raised when it would cut into the returned page, and a cut pool is recorded `truncated` so a consumer knows not to compute pool recall from it. Validated `> 0` (`[observations] max_candidates`) | `100` |
| `COMEMORY_TUNE_MIN_GOLDEN` | Test hook lowering `comemory tune` / `comemory bandit`'s minimum-golden-pairs floor; not a tuning knob | `10` |
| `COMEMORY_REINFORCE_SEARCH_EDIT_DAYS` | Lookback days for search→edit auto-reinforcement provenance (`auto_search_edit`). Validated `≥ 1` | `7` |
| `COMEMORY_DISABLE_ACCESS_TRACKING` | Test hook (truthy) disabling `search` / `context` access tracking + `retrieval_log` writes for one run, so a stability harness can drive the binary repeatedly without each query mutating `access_count` / `last_accessed`. Only a head page is bumped since #201, so a pure deep-paging loop no longer needs this hook; what it still buys a harness is a frozen *first* page, whose rows a repeated query does re-reinforce. Not a user knob | `false` |
| `COMEMORY_SYNC_PUSH_ON_SAVE` | Push the outbox inline after `save` / `delete` (`[sync] push_on_save`). This is what lets continuous sync stop depending on a resident process | `true` |
| `COMEMORY_SYNC_CODE_INDEX` | Push the code index of every indexed repo alongside memories (`[sync] code_index`): paths, blob OIDs, symbol names and line ranges, `imports` / `co_changed` edges — never source text. On by default so the console's graph fills in right after `auth login`; `skip_repos` still withholds a repo | `true` |
| `COMEMORY_SYNC_PUSH_ON_SAVE_TIMEOUT` | Budget for that inline push only (`[sync] push_on_save_timeout`), far below the 30s every other platform call takes: a save must return on a captive-portal network. Validated `> 0` — disable the hook with `push_on_save = false` rather than a zero budget | `2s` |
| `COMEMORY_GIT_AUTO_SYNC` | `true`/`1` to enable best-effort git commit + push after a save. Also a file key: `[git] auto_sync` / `[git] remote` in `config.toml` (written by `PATCH /api/v1/memory-stores/default`); env wins over file | `false` |
| `COMEMORY_EMBED_HINT` | Free-form identifier of the embedder you used (e.g. `ollama:nomic-embed-text`). Surfaced by `comemory doctor`; never consumed as a switch. | unset |
| `COMEMORY_EMBED_CMD` | Embed command used by `comemory serve`'s `POST /api/v1/doctor/reembed` (re-vectorize memories/code server-side). Run as `sh -c <cmd>`; reads the text on stdin, must emit `{"embedding":[..]}` on stdout. `serve --embed-cmd` overrides it. Unset → reembed answers `503 embedder_unavailable`; lexical search always works. | unset |
| `COMEMORY_RANK_DECAY` | ACT-R decay exponent `d` in `ln(n) − d·ln(days+1)`. Must be ≥ 0. Higher → older memories decay faster. | `0.5` |
| `COMEMORY_RANK_PRIOR_CLAMP` | `"lo,hi"` bounds applied to the activation, feedback, quality, and PageRank boost multipliers (the fixed `0.2` supersede penalty intentionally bypasses the clamp). Both finite; lo > 0, lo ≤ hi. | `0.5,2.0` |
| `COMEMORY_RANK_MMR_LAMBDA` | MMR relevance-vs-diversity trade-off in `[0.0, 1.0]`. `1.0` = pure relevance; `0.0` = pure diversity. | `0.7` |
| `COMEMORY_RANK_NEAR_DUP_HAMMING` | SimHash Hamming radius for near-dup detection (save-time advisory + diversify collapse). Must be ≤ 64 (SimHash is 64-bit). | `8` |
| `COMEMORY_PRUNE_MIN_ACTIVATION` | Activation floor (ACT-R scale) below which a memory is prune-eligible. | `-2.0` |
| `COMEMORY_PRUNE_MIN_FEEDBACK` | Beta-feedback ceiling (range `[0.0, 1.0]`) at or below which a memory is prune-eligible. | `0.25` |
| `COMEMORY_PRUNE_BELOW_QUALITY` | Quality threshold (1..=5); memories at or below this value are prune candidates (used together with activation + feedback floors). | `2` |
| `COMEMORY_PRUNE_SUPERSEDED_GRACE_DAYS` | Grace window (days) before a superseded-and-never-accessed memory becomes prune-eligible; protects freshly-rebuilt DBs whose supersede edges all carry rebuild-time timestamps. | `7` |
| `COMEMORY_RELEASES_URL` | Test hook: the release base `comemory upgrade` and `install.sh` resolve `latest` and download assets from (`<base>/latest`, `<base>/download/<tag>/<asset>`); the suite points it at a loopback stand-in (`tests/common/release_server.rs`). Not a user knob. | `https://github.com/Falconiere/comemory/releases` |
| `COMEMORY_API` | Platform API base for `comemory auth` (trailing slash stripped). Overridden by `auth login|status --api-url`. | `https://api.comemory.io` |
| `COMEMORY_API_KEY` | Optional override of the `auth.json` org-key secret (same idea as CI without writing the file). | unset |
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
`memory_vec` / `code_vec` vec0 DDL (`migrations/0002_v2_tables.sql`)
at migration time and are not env-configurable: a divergent env value
would silently disagree with the vtab and surface as `VecDimMismatch`
at first insert. Change the literal in the DDL if you need a different
dim.

CLI flags `--data-dir` and `--json` are global and can appear before or
after the subcommand.

## Memory Data Model

Frontmatter schema v1, defined by `src/domains/memories/frontmatter.rs::Frontmatter`:

```yaml
---
id: a1b2c3d4                  # 8-hex prefix of SHA-256(body.trim_end()); unique per store —
                              # a same-id different-body save is refused (IdCollision), never absorbed
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
2a. **Prior lookup** (`MemoryStore::prior`, `src/domains/memories/prior.rs`), run
   once the vector is parsed and the data dir exists but BEFORE the
   database is opened — so ahead of step 2's dim guard: what
   `memories/{id}-*.md` — else `.trash/{id}-*.md` — already holds for the
   content-derived id. A prior whose `content_hash` differs is a 32-bit
   collision → `Error::IdCollision` (HTTP `409 id_collision`, CLI exit 65)
   with nothing written and no DB touched. A matching prior contributes its `created`, so a
   replay never re-stamps the markdown, and makes the response
   `created: false`. The same lookup backs `domains::sync::exchange::import_state`'s
   pulled-record collision rule.
2b. **Near-duplicate check** (best-effort, advisory): scan live `memories`
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
     `<repo>:<path>` / `<repo>:<path>:<symbol>` mentions; `memories::mirror`
     hands the result to `store::memory_row`, which writes the
     `ReferencesFile` / `ReferencesSymbol` rows into `edges`. Missing
     `code_symbols` rows are tolerated — `comemory index-code` fills them
     in later. URLs (`://`, `git@host:`) and path expressions behind a bare
     scheme (`file:/tmp/x.db`, `sqlite:./x.db`, `file:../x.db` — any
     captured path starting with `/`, `./` or `../`) are never references;
     migration `0018` deletes the ones older binaries minted.
5. Best-effort git auto-sync via `git_utils`, only when
   `COMEMORY_GIT_AUTO_SYNC` is enabled.
5a. **Inline cloud push** (`cli::save` only, not `domains::memories::save`): when
   `[sync] push_on_save` is on and `auth.json` is usable, drain the `sync_log`
   outbox to the platform under `push_on_save_timeout`. Never fails the save —
   offline, unauthenticated and refused all leave the entry in the outbox for
   the next `sync` / `watch` / daemon cycle. It lives in `cli::` because
   `domains::memories::save` also runs inside `comemory serve`, where
   `reqwest::blocking`
   panics on drop and pushing a tenant's memories outward would be wrong.
6. Answer `{id, path, created, duplicate_of?, warnings?}`. `created: false`
   is the idempotent replay (same body re-saved, metadata overwritten
   last-writer-wins, or a trashed id revived); TTY prints `updated <id>`
   instead of `saved <id>`. Stated contract, pinned by
   `tests/cli__save_3.rs` and `tests/serve__routes__memories__write.rs`,
   documented in `docs/guides/http-api.md` § Save contract.

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
   Per-command contracts live in `tests/cli__<cmd>.rs`. Multi-command
   user journeys live in `tests/cli_scenario_*.rs` and share
   `tests/common/cli_bin.rs`; each journey has an `/api/v1` twin in
   `tests/serve_scenario_*.rs` over a real `comemory serve`, sharing
   `tests/common/serve_bin.rs`. `docs/scenarios/` is the human-readable
   test plan (one file per subcommand, every flag, its HTTP route, which
   test covers it); `tests/cli_scenario_catalog.rs` walks the built
   `Cli::command()` plus the live `GET /api/v1/commands` and fails when a
   subcommand, flag, positional, or route is missing from its file, when a
   flag has no scenario citing a test, or when a cited `tests/…rs::fn`
   does not exist.
4. **Otherwise** -> colocated, in a sibling `tests/` folder beside the module
   under test:

       src/store/migrate.rs          production module
       src/store/tests/migrate.rs    its tests
       src/utilities/simhash.rs      a shared primitive
       src/utilities/tests/simhash.rs  its tests

   The production file names its test files with a one-line include, which makes
   it the index of its own suite:

       #[cfg(test)]
       #[path = "tests/migrate.rs"]
       mod tests;

Keep each `tests/` tree **flat**. A suite that outgrows one file splits into
`<name>_2.rs` (or `<name>_v4.rs` for a version-scoped suite) beside it, and the
production file gains a second include (`mod tests_2;`). Tests are exempt from
the 300-line ceiling, so splitting is a readability choice, not an obligation.

Measured today (2026-09-02): 194 colocated `src/**/tests/*.rs` files, 106
crate-root `tests/*.rs` surfaces (61 CLI, 8 HTTP journeys, 80 `assert_cmd`, 3 `insta`).

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
scripts/architecture-check.sh    # module ownership and dependency policy
scripts/store-chokepoint-check.sh # rusqlite confined to src/store/, ratcheted
                                  # against store-leak-baseline.txt
scripts/typos-check.sh           # typos
scripts/cli-docs-check.sh        # docs/cli-reference.md vs the real --help output
scripts/migration-check.sh       # shipped migrations/*.sql is byte-identical
                                  # to its content at the first release tag that
                                  # carried it, under migrations/ or the pre-v0.29
                                  # src/store/ location, matched by basename
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
| Shipped migration SQL is immutable | `scripts/migration-check.sh` (git-tag-dependent; compares each `migrations/*.sql` against its first release tag, accepting the pre-v0.29 location under `src/store/` for the same basename) |
| Duplication ratchet | `scripts/dup-check.sh` against `dup-baseline.txt`; it **enforces** the `similarity-rs` build named by its `SIMILARITY_RS_VERSION` constant, hard-failing on any other version (see `docs/dup-debt.md`) |
| rusqlite confined to `src/store/` | `store-leak-baseline.txt`, `scripts/store-chokepoint-check.sh` |

Three further gates need a separately installed tool, so they are deliberately
outside `check-all.sh`'s `GATES` array — and all three run both in `just qa` and
as their own steps in `.github/workflows/test.yml`:

| Gate | Tool, and the constant that pins it |
| --- | --- |
| `scripts/deny-check.sh` (licence + advisory policy) | `cargo-deny`, `CARGO_DENY_VERSION` |
| `scripts/dup-check.sh` (duplication ratchet) | `similarity-rs`, `SIMILARITY_RS_VERSION` |
| `scripts/machete-check.sh` (unused dependencies) | `cargo-machete`, `CARGO_MACHETE_VERSION` |

Each constant lives on one line in its own script and is the single source of
truth: the workflow *derives* the version it installs from that line instead of
repeating it, so the workflow and the gate cannot drift, and bumping the
constant is the only edit a version bump needs. The two cases are handled differently, and the difference
matters: a **version mismatch** is a hard failure everywhere, CI and local
alike, because a verdict from another build is not the verdict CI will reach;
a **missing tool** fails in CI but locally prints a loud `[warn]` saying the
gate did not run, names the pinned install command, and **exits 0**. So a green
`just qa` on a machine without the tool means "this gate was skipped, loudly",
not "this gate passed" — CI is the run that decides.

Two of them assert that their tool actually ingested the tree, because a gate
that reports success while checking nothing is this repo's recurring failure
mode: `dup-check` parses `similarity-rs`'s `Checking N files` line and fails
when `N` differs from the file count it handed over, and `machete-check` runs
`cargo-machete` over a copy of this crate's own `Cargo.toml` first and fails
unless that canary reports unused dependencies — the tool exits 0 with a
success message when handed a directory containing no manifest at all.
`deny-check` asserts that cargo-deny read this repo's `deny.toml`, gathered a
crate count floored against `Cargo.lock`, fetched the advisory database, and
reported all four of its checks.

`scripts/test-run.sh` runs the nextest suite. A task is not "done" until
`scripts/check-all.sh` exits 0.

## Distribution

- `curl … https://github.com/Falconiere/comemory/releases/latest/download/install.sh | sh`
  — the repo-root `install.sh`, hand-written (POSIX sh, ≤ 300 code lines like
  every other file), attached to every release by `release-finalize.yml`
  (which also smoke-runs it against the fresh release on the Linux runner).
  Platform detection, `latest` via the release redirect, SHA-256 sidecar
  verification (never skipped), a `--version` run of the downloaded binary
  before the atomic rename, in-place replacement of the `comemory` already on
  `PATH`, once-only rc-file `PATH` line. Its `--version` / `--dir` /
  `--no-modify-path` / `--quiet` flags are a contract: `comemory upgrade`
  runs it with exactly those. The command with its hardening flags is spelled
  out in README § Install.
- `https://get.comemory.io/pkg/comemory/install` — a 302 served by the
  `comemory-prod` Cloudflare Worker on the comemory.io platform; it still
  points at cargo-dist's `comemory-installer.sh` until that worker is
  repointed at the `install.sh` asset above.
- `comemory-installer.sh` — the cargo-dist generated installer, still
  published on every release (`installers = ["shell", …]`); no pinning, no
  in-place upgrade, skips the checksum on stock macOS.
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
the version + rewrites `CHANGELOG.md` from conventional commits — but only when at
least one commit since the last tag would render a changelog line (`release_commits`;
a `ci:`/`docs:`/`chore:`-only push opens nothing); merging it pushes the
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

Every deviation below is deliberate and documented here. `AGENTS.md` carries
the repository guidance, and `CLAUDE.md` points to it. None weakens the kit's
intent; several make the local rule strictly stronger than the one it replaces.

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
- **D2 — `fileSize.max` is 300, not the kit's 500, with exactly one
  `fileSize.overrides` entry: 400 for `.github/workflows/release.yml`.**
  comemory's module decomposition is designed around 300 code lines. CORE
  permits a stack to add rules but never relax one; 300 is stricter, so this
  is compliant. The one override is scoped to `.github/workflows/release.yml`
  and raises the ceiling for no other path — every other file in the repo,
  `.github/workflows/release-finalize.yml` and `.github/workflows/test.yml`
  included, is still checked at 300. `release.yml` is generated by
  `cargo dist init`, which emits roughly 310 code lines; the repo does not
  choose its length and cannot split it, so the ceiling was unenforceable there
  and
  only ever fired in lefthook's file mode (the repo-mode sweep scans
  `srcRoot: src` and never saw it), blocking any commit that touched the
  workflow. Every hand-written workflow stays under the 300 default — the
  next largest is `.github/workflows/release-image.yml` at 139. The override
  stays even though the file currently measures 295 — it dipped under 300 only
  because the hand-maintained `await-review` job was deleted from it, and the
  next `cargo dist init` puts the generated bulk back. Re-measure
  any of these the way the gate does, blanks and comments excluded:

      awk 'NF && $1 !~ /^#/' <file> | wc -l

- **D3 — `barrelNames: ["mod.rs"]`** is inherited unchanged from the pinned
  Rust conventions, not a local deviation. It makes the `<dir>.rs` beside
  `<dir>/` layout permanent and machine-checked.
- **D4 — `src.nested` replaces the starter map** with the exact, project-specific
  folder policy below. It retains the kit's test separation while replacing its
  generic `model`/`service`/`store` domain shape with comemory's staged
  capabilities and delivery adapters:

  - `domains`: `architecture`, `memories`, `code`, `documents`, `graph`, `retrieval`, `learning`, `sync`, `capture`, `maintenance`, `integrations`
  - `domains/*`: `ast`, `cloud`, `consolidation`, `doctor`, `document`, `evaluation`, `exchange`, `index_code`, `install`, `memory_store`, `rebuild`, `repos`, `retention`, `setup`, `source`, `unified`, `upgrade`, `tests`, `proptest-regressions`
  - `store`: `tokenizer`, `migrate`, `tests`
  - `cli`: `output`, `tests`, `proptest-regressions`, `setup`
  - `serve`: `routes`, `jobs`, `tests`, `proptest-regressions`
  - `serve/routes`: `memories`, `maint`, `tests`, `proptest-regressions`
  - `*`: `tests`, `proptest-regressions`

  `serve` remains the HTTP adapter, `mcp` is the stdio MCP adapter, and
  `store` remains the SQLite exception; no template name controls their
  project ownership.
- **D5 — `src.requireReadme` replaces the pinned single-`domains` rule during staging with exactly 43 folders.**
  The `domains` entry returned with #167, the first real capability domain to
  land, and every capability folder gains its own entry as it lands. It names
  folders, not files: each listed grown folder has a `README.md` that indexes
  its contents, while a single-file module is listed in its parent folder README
  and documented by its module doc. The configured list, rather than a prose
  count, is the source of truth.
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
  `cli-docs-check`, `coverage-check`, `eval-check`, and the three tool-dependent
  gates `dup-check`, `machete-check` and `deny-check` — each pinned to one tool
  build by a constant in its own script, and each wired into the `test`
  workflow. None has a kit equivalent; all guard
  comemory-specific failure modes. They layer around the guardrails step, not
  in place of it. Mutation testing is no longer among them: its workflow ran
  a nightly full sweep and a PR-scoped job that had been disabled, and it was
  removed. `just mutation` still runs the full sweep locally through
  `scripts/mutation-check.sh`, against `.cargo/mutants.toml` and the
  `tests/golden/mutant-baseline.md` baseline — the tool stays, only the
  scheduled CI job is gone.
- **D9 — `tests/common/` stays at the crate root** and is bridged into the lib
  test crate via `src/test_common.rs`, so colocated unit tests and crate-root
  integration tests share one copy of every fixture (Binding Rule 1). Its
  `#![allow(dead_code)]` is the migration's one sanctioned exception,
  exempted by filename in `no-allow-attribute.yml`.
- **D10 — `extern crate self as comemory;` is added to `src/lib.rs`.** This
  lets every colocated test file keep `use comemory::...` imports identical
  to the crate-root suite's, so a test reads the same wherever it lives.
- **D11 — 106 of 300 test files remain crate-root integration tests**
  (61 CLI surfaces, 80 driving the real binary via `assert_cmd`, 3 owning an
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

## Agent integration

`comemory install claude` and `comemory install codex` install the bundled skills
and hooks through the host plugin manager. Use `--dry-run` to preview. Prefer
MCP `find` with `k: 3`, then `show` selected memories; `context` returns full
bodies without a token cap. Stop reminders summarize shared repo activity and
never block a session. Transcript capture currently supports Claude Code only.
The integration is owned here and requires no toolu plugin. See
[installation and migration](docs/guides/agent-integration.md).
