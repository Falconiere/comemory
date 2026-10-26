# AGENTS.md

Canonical agent guidance for this repository. `CLAUDE.md` is removed.

## Project Overview

`comemory` is Rust CLI that fuses engram-style developer memory, grepai-style
semantic code search, and ast-grep AST patterns into one binary, knit
together by SQLite-backed store (memory + code rows + edges). It is 
**standalone agentic-RAG toolbox**, invoked from command line, over its
loopback HTTP API (`comemory serve`), or as MCP stdio server (`comemory
mcp`) that speaks same command cores as those two — still no in-process
LLM. Everything runs locally: markdown is source of truth and one SQLite
file (`comemory.db`) backs FTS5 + `sqlite-vec` + edges.

## Conventions that are easy to misread

- **`requireReadme` names folders, not files.** `src/<module>/README.md` is 
  per-file index for files inside `src/<module>/`. single-file module
  needs no README of its own; it is listed in its parent folder's README and
  documented with its `//!` module doc. `guardrails.config.json` lists exactly
  folders that require README.
- **`POST` is not automatically mutating.** `RouteEntry::mutating` controls
  whether read-only gate refuses route that writes to store.
  `POST /jobs/{id}/cancel` is deliberately `mutating: false`, so runaway job
  can be stopped on read-only server.

## Where to start reading

- `src/main.rs` / `src/lib.rs` — binary entry and library surface
- `src/cli.rs` / `src/cli/save.rs` — command dispatch and save flow
- `src/domains/retrieval/pipeline.rs` — search orchestration
- `src/store.rs` — SQLite connection management
- `docs/architecture.md` — full architecture explanation

## Architecture

- **Source of truth:** markdown files with YAML frontmatter at
  `~/.comemory/memories/{id}-{slug}.md` (override with `COMEMORY_DATA_DIR`).
- **Single SQLite file:** `~/.comemory/comemory.db` with `memories`,
  `memory_fts` (FTS5), `memory_substring` (external-content trigram FTS5), `memory_vec` (`sqlite-vec` `vec0`), `code_symbols`,
  `code_fts`, `code_vec`, `edges`, `schema_meta`, plus stats / repo-marker
  tables and — when candidate capture is enabled — three
  candidate-observation tables `candidate_query_observations`,
  `candidate_observations` and `candidate_judgments` (#209), and 
  fourteen project tables (`projects`, `project_work_items`, … — #325) plus
  engine-only `project_transfer_bindings` (#342), all registered leaf
  first, each with its transfer class, in
  `store::schema_projects::PROJECT_TABLES`, plus `project_changes`, 
  body-free change feed kept outside that registry (#324).
  `rusqlite 0.40` with `bundled` + `load_extension` features.
- **Declared schema (toolu-orm 0.12):** every table in `comemory.db` is 
  `#[table]` / `#[fts5_table]` / `#[vec0_table]` struct in
  `src/store/schema_*.rs`, assembled by `store::schema::registry()`.
  `just migration <name>` diffs structs against
  `migrations/<newest>.snapshot.json` and writes next
  `migrations/NNNN_<name>.sql` plus SHA-256 journal entry;
  `just migration-journal` journals hand-written file when change is
  easier to write than to declare (data backfills, rebuilds). Runtime APPLY is
  unchanged: `store::migrate` runs every file with `execute_batch`, keyed by
  `schema_meta` markers; no `_migrations` table. See
  `docs/guides/schema-migrations.md`.
- **Runtime queries:** use declared toolu-orm table builders and private
  `store::orm` execution bridge, preserving native SQLite errors and caller
  transactions. Unsupported SQL stays inside `store/` and is tracked in
  `docs/guides/runtime-orm.md`; never substitute `INSERT OR REPLACE` for 
  row-preserving upsert. `stats_counts::Corpus` selects scoped counters without
  passing SQL predicates across store boundary. Historical migrations and
  independent test fixtures remain SQL.
- **Edges:** flat `(src_kind, src_id, edge_kind, dst_kind, dst_id)` rows
  (plus integer `weight`) in `edges` table replace v0.1 kuzu
  graph. v6 adds code-graph kinds: `co_changed` (mined from git history)
  and `imports` (per-language import resolution), feeding materialized
  PageRank on `code_symbols.rank_score`. Recursive CTEs handle multi-hop
  traversal.
- **AST extraction:** `ast-grep-core 0.45` + `ast-grep-language 0.45` (rust,
  typescript, javascript, python, go only).
- **Vectors are BYO.** No in-process embedder. Callers pass vectors via
  `--vector` (CSV) or `--vector-stdin` (JSON `{"embedding":[..]}`). sample
  Ollama wrapper ships in `scripts/comemory-embed.sh`.
- **Output:** TTY via `owo-colors`, JSON via `serde_json`. Exit codes follow
  `sysexits.h`.
- **No in-process LLM.** All ranking is deterministic (RRF fusion of FTS5 +
  `sqlite-vec`, tiered lexical fallback ladder ending in mined learned
  expansions, edge walks).

Migration 19 adds graph/history/listing indexes and `memory_substring`. Its
triggers maintain substring index for every memory write; listing retains
literal `LIKE` verification and scan fallback for queries under three characters.
external-content FTS5 table stores trigram postings and reads body text from
`memories.body`, without storing another body copy.

Managed sync uses `repository-policy-v1` in `X-Comemory-Sync-Protocol` for
both legacy and replica transports. `replica-v1` belongs to replica bodies
and manifests; never substitute it for policy header. Response protocol
and policy revision echoes remain mandatory. opt-in real-platform
regression is documented in `docs/guides/managed-replica-testing.md`.

## Key Commands

```bash
cargo install --path .          # build + install the binary locally
comemory setup                  # detect + apply this machine's/repo's onboarding
comemory completions --install  # install + register bash/zsh/fish/powershell completions
just check                      # umbrella gate (alias of scripts/check-all.sh)
just test                       # cargo nextest run --all-features
just qa                         # check-all + cargo-deny + dup-check + machete + install.sh libc gate (docker)
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
comemory install-hooks --repo P    # hook a repo once: every HEAD move there indexes + syncs it (no cd afterwards)
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
bash scripts/deploy.sh --tag vX.Y.Z --change-log "..." [--dry-run]  # local stable release while Actions are off (docs/release.md §3b)
comemory auth login|status|logout  # org-scoped cloud login; `login` also runs the first sync
comemory sync                      # drain the bound key both ways until caught up (replica-v1, or the old protocol + code index)
comemory sync --action auto        # the pass hooks fire: refresh every stale hooked repo, then drain (no login needed)
comemory watch                     # follow the workspace channel; pull on every nudge
comemory capture session --path F  # redact + POST a session receipt (or --dry-run)
comemory capture sources           # show platform capture consent (CLI cannot grant)
comemory capture install-hook      # Claude Code SessionEnd → capture session --from-hook
comemory distill --session-id <id> --transcript <path>  # propose candidates from explicit saves
comemory mcp                       # stdio MCP adapter: the nineteen-tool catalog over JSON-RPC
comemory project create|show|list  # engine-owned project charters, offline (epic #261)
comemory project archive|restore|pause|resume <id> --expected-version N  # lead-only lifecycle (#328)
comemory project export|import     # move one project between data directories as a bundle (#342)
comemory recall-status             # tracked queries, verdicts, saves and pending recalls for a repo
just migration <name>              # struct diff → migrations/NNNN_<name>.sql (+ snapshot + journal)
just migration-journal <file>      # journal a hand-written migrations/NNNN_<name>.sql
just migration-adopt               # restate the newest snapshot from src/store/schema*.rs
```

## Binding Rules (apply to every contribution)

comemory follows toolu-conventions Rust stack
(`github.com/Falconiere/toolu-conventions`). These rules are that kit's, plus
comemory's stricter local ceilings. Deviations are enumerated under
"Deviations from toolu-conventions" — nowhere else.

1. **No duplication / redundancy.** Shared logic is extracted into helper.
   Enforced by `scripts/dup-check.sh` (which excludes test trees) and review.
2. **No barrels — no `mod.rs`.** module that grows into folder keeps its
   file beside it: `src/store.rs` declares `mod migrate;`, `src/store/migrate.rs`
   holds it. file whose only content is `pub use` re-exports is barrel and
   is banned; `pub use` is legitimate only in `src/lib.rs`, to shape crate's
   public API. Enforced by `no-barrels` guardrails check
   (`barrelNames: ["mod.rs"]`).
3. **One responsibility per file; filename matches content.** `snake_case`,
   named after file's primary item (`code_row.rs` holds
   `struct CodeSymbolRow` and its writers). Enforced by `filename-case`
   guardrails check.
4. **Size ceilings.** 300 code lines per file in `src/` (blanks and comments
   excluded; tests exempt) — stricter than kit's 500 by choice, because
   comemory's module decomposition is built around it. 100 lines per function.
   Both are DECLARED in `guardrails.config.json` (`fileSize.max`,
   `functionSize.max`) and enforced by `file-size` guardrails check and
   `clippy::too_many_lines = "deny"` respectively.
5. **Zero errors, zero warnings, no silencing.** Clippy over all targets and all
   features with `-D warnings` must be clean. No `#[allow(...)]` in production
   code — lint is either house policy (declared once in
   `Cargo.toml [lints]`) or it is fixed. No `.unwrap()` / `.expect(...)` /
   `panic!` / `todo!` / `unimplemented!` / `println!` / `eprintln!` / `dbg!`
   outside tests and benches — return `Result` and propagate with `?`, and route
   diagnostics through `tracing`. Every `unsafe` block and every `unsafe fn`
   carries `// SAFETY:` line in comment block directly above it. Enforced
   by `Cargo.toml [lints]` plus two project-local ast-grep rules in
   `scripts/guardrails/patterns/rust/`.
6. **Tests never share file with production logic, and colocate by default.**
   No `#[cfg(test)] mod tests { ... }` body ever appears in `src/` file. See
   "Testing" for placement rule. Enforced by `no-inline-test-module`
   ast-grep rule.
7. **Doc line on every module and public item.** `//!` at top of every
   module, `///` on every `pub` item. Enforced by `missing_docs = "warn"` under
   `-D warnings`.
   **folder** README is separate rule and applies to folders only:
   `src/<module>/README.md` indexes files inside `src/<module>/`. 
   single-file module — `src/store/memory_purge.rs`, `src/config/patch.rs`,
   `src/domains/graph/neighbors.rs` — has no folder and therefore needs no README of
   its own; it is listed in its parent folder's README, and its `//!` doc is
   its documentation. `guardrails.config.json`'s `src.requireReadme` names
   folders, and `scripts/guardrails/run.sh` is what enforces it.
8. **Docs in sync.** change to user-facing surface (CLI flags, public API,
   config, env vars) updates `README.md`, `AGENTS.md`, `docs/` and module's
   own `src/<module>/README.md` in same change.
   `scripts/cli-docs-check.sh` enforces `docs/cli-reference.md` half
   mechanically. generator discovers visible top-level commands from
   `--help`; `tests/cli_scenario_catalog.rs` independently checks its section
   inventory against clap, so regeneration cannot silently omit command.
9. **Real data, no mock-data tests.** test that only proves mock returns
   what mock was told to return is banned — it hides integration breakage.

10. **`rusqlite` is confined to store module.** `src/store.rs` and
   `src/store/` are only production code that may name driver; every
   other module takes `store::Connection` / `store::Transaction` and calls 
   `store::<table>` helper. No `Statement`, `Rows` or `rusqlite::Error` may
   cross that boundary in return type — read returns owned `Vec<T>`,
   `Option<T>` or scalar, so `QueryReturnedNoRows` never becomes caller's
   control flow. `src/errors.rs`'s `Error::Sqlite(#[from] rusqlite::Error)` is
   single permanent exception, kept deliberately: removing `#[from]` would
   force `.map_err(...)` at every `?` inside `store/`. Enforced by
   `scripts/store-chokepoint-check.sh` against `store-leak-baseline.txt`, 
   two-sided ratchet — it fails both when count rises above baseline
   and when it drops below it without baseline being lowered. baseline
   is `0`; it must never go up.

   **boundary is two-way (#177).** `store/` must not call domain
   service, algorithm or derivation either: it persists what it is given. 
   domain resolves whatever row needs BEFORE write and passes it in as
   owned or borrowed row data (`store::CreatedWindow`, `store::MemoryLinks`),
   and post-write refresh of derived state is caller's step, never 
   writer's. store file may name domain **type** as passive data input —
   `Frontmatter`, `Ref`, `References` — and only through explicit
   `passive_store_models` allowlist in `scripts/architecture-policy.json`,
   whose targets must be types, never functions or modules. Enforced by
   `scripts/architecture-check.sh` (`store service dependency`); 
   `store_callbacks` allowlist that used to carry exceptions is now
   empty.

one command that must be green before every push:

    bash scripts/check-all.sh      # or: just check

## Code Style

- `rustfmt` stable-only options — **4-space indent**, 100-column line length,
  edition 2024 (`rustfmt.toml`). Nightly-gated knobs (`imports_granularity`,
  `group_imports`, `wrap_comments`,...) are deliberately omitted so `cargo
  fmt` is deterministic on stable toolchain CI uses.
- **≤300 code lines per `src/` file** (blanks/comments excluded; see
  Binding Rule 4) — split into submodules before crossing it.
- **No `mod.rs`.** module that outgrows one file is `src/<name>.rs` beside
  `src/<name>/`, never `src/<name>/mod.rs`.
- **One primary item per file, filename matches content.** file named after
  type or function holds that item and its direct helpers, not unrelated
  second concern.
- Lint policy is declared once, in `Cargo.toml [lints.rust]` / `[lints.clippy]`
  plus `clippy.toml` — never per-call-site `#[allow(...)]`. Run it with
  `cargo clippy --all-targets --all-features -- -D warnings`
  (`scripts/lint-check.sh`).
- Doc comments (`///`) on every public item, `//!` at top of every module.
- `Result<T>` alias from `crate::prelude::*`; errors flow through 
  `Error` enum in `src/errors.rs`.
- Use `tracing` for diagnostics, never `println!` / `eprintln!`.

## Module Map

Every folder listed with trailing `/` below carries its own
`src/<module>/README.md` — per-file index for that folder, kept current by
whoever last touched file there. table below is cross-module
narrative; folder `README.md` is authoritative file-by-file list.

| Module | Responsibility |
|--------|---------------|
| `cli/off_runtime.rs` | `off_runtime` — runs blocking platform I/O on scoped thread. `main` is `#[tokio::main]`, so every subcommand body sits inside async context, and `reqwest::blocking` panics on drop there (`Cannot drop a runtime in a context where blocking is not allowed`). `block_in_place` does not help — runtime handle stays current, which is what reqwest objects to. Used by `cli::auth` and `cli::sync`; `serve` needs nothing (it already runs its command cores in `spawn_blocking`) |
| `cli/sync_auto.rs` | `comemory sync --action auto` — CLI half of `domains::sync::auto`, dispatched by `cli::sync` before any login is required. With required coordinator (#257) it sends one wake over control socket and returns; only under `COMEMORY_SYNC_DAEMON=0` harness switch does it fall back to running pass off runtime itself (`domains::code::hooked_refresh` is local half of that fallback — index `--path` checkout, re-index every stale hooked repo; passes serialize on `sync.lock` and coalesce through one-slot `sync-auto.queue`). Prints nothing unless `--json` either way (its callers are git hooks and agent hooks) |
| `cli/sync_daemon.rs` | `comemory sync daemon {ensure,status,restart,repair,stop,uninstall,run}` — required resident coordinator's lifecycle (#257); `install`/`start` are hidden deprecated aliases of `repair`/`ensure`. `run` is supervisor's foreground entry point (`domains::sync::daemon::run_foreground`); every other verb calls `domains::sync::daemon::ensure`/`status_view` off runtime |
| `cli/daemon_preflight.rs` | Runs first in `cli::run`, before every ordinary command: exhaustive match over `Cmd` classifies each as `Exempt` (lifecycle/introspection — never touches coordinator), `BestEffort` (`auth logout` — verifies/repairs but never fails command), or `Required` (everything else, including `watch`/`mcp` — fails with named local-service error, exit 69, if no coordinator can be reached). no-op under `COMEMORY_SYNC_DAEMON=0` |
| `cli/watch.rs` | `comemory watch` — with required coordinator running, attaches to it instead of holding its own connection: `--once` subscribes then asks for one `catch_up`; following instead reports one `pulled` per `pass_finished` event, re-attaching (jittered backoff, 1s → 30s) on dropped subscription, coordinator restart included. Only under `COMEMORY_SYNC_DAEMON=0` does it hold platform's workspace channel itself, as before #257: mints 60-second ticket, holds it open (`tokio-tungstenite`, async on existing runtime), and drains pull direction (`domains::sync::drain`, pass after pass until no `more`) on every `hello`/`change` frame — nothing is read out of frame except that it arrived, so missed one costs latency and duplicate costs empty pull |
| `cli/` | clap subcommand entry points + top-level dispatcher in `cli.rs`, plus `output/` (TTY/JSON writers, moved in from top level by #178), `completion_script` (completion-script generation for both `comemory completions` and `GET /api/v1/completions` — one sanctioned HTTP-to-CLI bridge: generating completion script *is* clap work, so it stays beside clap definition it reflects rather than being forced into capability, and `serve::routes::meta` imports `cli::{Cli, completion_script}` for exactly that and `GET /commands` inventory), `completion_install` (idempotent per-user Bash/Zsh/Fish/PowerShell installer behind `comemory completions --install`; `install.sh` and developer installer call it after placing binary), and `pagination` (flattened `--limit`/`--offset` clap struct). shared date-flag layer moved to `utilities::when` with #166; `edges` is fourth free-text surface (`comemory edges <query>` — lexical search over `edge_fts`, self-healing empty index on first use); `graph` keeps only clap surface and `--format` renderers since #170 |
| `cli/setup/` | terminal surface of `comemory setup`: `wizard` (cliclack prompts — deliberately thinnest file in command, holding no decisions, so one file pty test would be needed for has nothing in it worth testing; cancellation arrives as `ErrorKind::Interrupted` and becomes `Ok(None)`, and all-satisfied plan short-circuits rather than building empty multiselect cliclack rejects) and `render` (non-interactive summary, written into `impl Write` so `tests/cli__setup.rs` snapshots it). `cli/setup.rs` owns clap `Args`, `Intent`/`Prompting` → `Mode` decision, and exit-code mapping |
| `utilities/embed.rs` | shared embed-command shell-out (single-file module, no children) — runs `COMEMORY_EMBED_CMD` / `--embed-cmd` as `sh -c <cmd>`, feeds query on stdin, parses `{"embedding":[..]}`. Consumed by `serve` (`POST /api/v1/doctor/reembed`, server-side re-vectorizing job; `GET /health` reports `embed_cmd_configured`). `EMBED_TIMEOUT` bounds stdout read rather than whole execution, so child that never drains large stdin can block caller on write |
| `serve/` | loopback `/api/v1` HTTP server (`comemory serve`, API-only — embedded web viewer was removed): axum `router` (mounts `routes::v1_router` behind path-aware `guard` middleware — enveloped `401`/`403` JSON on `/api/v1/*`, plain text on any other `/api/*` path — and 5 MiB `BODY_LIMIT`), `scope` (`RepoScope` — per-request default `repo` filter: `X-Comemory-Repo` header first, server's `--repo` second, never overriding explicit parameter), `security` (session token generation/matching and loopback Host guard; canonicalize-and-contain checks `resolve_within` and `contain_abs` are transport-neutral and live in `utilities::path_containment`), `envelope` (`{ok,data,meta}` / `{ok,error,meta}` `/api/v1` response envelope plus one `Error → (StatusCode, code)` mapping table every HTTP error and every failed job's `{code, message}` derives from, with optional structured `error.details`). `routes/` and `jobs/` are documented in their own rows below |
| `mcp/` | `comemory mcp` — stdio MCP adapter, third delivery surface beside `cli` and `serve`, sharing `utilities::blocking::run_blocking` and `utilities::error_code::classify` with `serve` rather than reimplementing either: `src/mcp.rs` (`McpOptions`, `serve` — open store, build `McpState`, run rmcp service over stdio until EOF), `catalog.rs` (`TOOLS` — curated nineteen-row `(name, command, mutating, description)` table `tests/mcp__parity.rs` walks against `Cli::command()`, ordered reads-then-writes: `find`, `search`, `search_code`, `context`, `show`, `list`, `edges`, `repos`, `recall_status`, `project_list`, `project_show`, four `architecture_*` tools, `project_propose`, `save`, `project_evidence`, `feedback`), `state.rs` (`McpState` — per-session `Arc<Mutex<()>>` gate + `Paths` + `Config` + default repo + `read_only`; `exec.rs` owns each call's live connection), `server.rs` (`ComemoryServer` — `#[tool_handler] impl ServerHandler`, `read_router() + project_router() + project_write_router() + write_router()`, `get_info`'s five-line `instructions`), `tools_read.rs` / `tools_projects.rs` / `tools_project_writes.rs` / `tools_write.rs` (`read_router`/`project_router`/`project_write_router`/`write_router` — twelve read tools, two project readers, `project_propose` and `project_evidence` writers, and three write tools, each calling matching `domains::<capability>::<cmd>::run` core and never reimplementing it; repo-scoped tools resolve scope first, project readers take none), `exec.rs` (`run` + `Access` — clone state, `run_blocking`, lock session and open/drop live connection inside that closure, refuse `Access::Write` outright under `--read-only`), `result.rs` (`into_tool_result` — `utilities::error_code::classify` into tool-level `structured_error` for every class but `Internal`, protocol `ErrorData` for `Internal`), `scope.rs` (`default_repo` — `--repo`, else cwd's main-worktree label via `domains::code::git_utils::repo_label_at`, else `None`), `params.rs` (`FeedbackParams` — one MCP-local parameter type: `confirmed_by_user` replaces core's `source`, so unstated verdict lands `implicit` and never enters golden harvest). Every `repo` parameter left unset resolves to session's default scope EXCEPT `repos`, which stays unscoped like `GET /api/v1/repos` since it is discovery surface naming which labels exist; `save` whose `repo` is still empty after that resolution is refused `repo_required` before store is touched. linked worktree is never repository — scope is always main worktree's basename. Owner `delivery::mcp` in `scripts/architecture-policy.json`: this folder never imports `cli` or `serve`, and no domain, `config` or `utilities` file may import it |
| `serve/routes/` | versioned `/api/v1` REST surface: `routes.rs` aggregates every resource's `table_entries()` into one route table (method/path/CLI-command/`mutating` flag — source of truth for read-only gate, `GET /commands`, and `tests/api__parity.rs`) and owns handler-layer helpers every resource shares — `query_response` (runs `domains::<capability>::<cmd>::run`, and connection-mutex guard it takes, entirely inside `utilities::blocking::run_blocking`'s `spawn_blocking` closure, never across `.await`), `respond`/`accepted` (envelope result / job-acceptance), `guard_mutating` (read-only-then-write-permit gate for synchronous mutating route: `405 read_only`, else `503 busy` + `Retry-After` on permit contention), `guard_job` (read-only-only gate for job-creating route — it always answers `202` immediately, permit contention only delays job itself), `require_confirm` (`confirmation_required` gate; its doc comment states read-only-outranks-confirm ordering, AC-19), `track_for` (shared access-tracking suppression for `search`/`search-code`/`context`). Per-resource files: `memories/` (`memories.rs` — `GET /memories`, `GET /memories/{id}`; `search.rs` — `GET|POST /memories/search`, `GET|POST /context`; `write.rs` — `POST /memories`, `DELETE /memories/{id}?confirm`, `POST /feedback`), `code.rs` (`GET|POST /code/search`, `POST /code/ast` with pre-run containment, job-backed `POST /code/index` and `POST /code/ingest` under its own 64 MiB body-limit layer), `graph.rs` (`GET /graph`/`GET /edges`, reusing `domains::graph::query`'s `build_code_graph`/`build_graph_page` pair — no second query path), `sources.rs` (`GET /sources`, job-backed `POST /sources`, `DELETE /sources?target=&confirm=`), `learning.rs` (job-backed `POST /eval` read-class, `POST /tune`/`POST /bandit` confirm-gated only when `apply`, `golden` containment before every other check), `maint/` (`maint.rs` — `GET /doctor`, `GET /consolidate`; `prune.rs` — `GET|POST /prune`, `POST /gc`, plus `split_confirm` — shared raw-body confirm-field extractor every confirm-gated route with real `Request` type reuses; `admin.rs` — `POST /mine`, `POST /hooks/install`, job-backed `POST /rebuild`, which replaces content in place through server's shared connection), `meta.rs` (`GET /completions`, `GET /commands` — clap-introspected route/command inventory), `stats.rs` (`GET /stats`), `repos.rs` (`GET /repos`), `find.rs` (`GET|POST /find` — its own resource because it is cross-domain, not memories sub-resource), `hooks.rs` (`GET /hooks` read, `POST /hooks` per-hook toggle behind read-only gate; NOT confirm-gated, since writing hook file is idempotent and reversible), `jobs.rs` (`GET /jobs`, `GET /jobs/{id}`, `GET /jobs/{id}/events` SSE with `status`/`progress`/`log` events, `POST /jobs/{id}/cancel`). console-api design (2026-09-01) added, as additional flat resource files: `overview.rs`, `search.rs` (console view over `find` + suggest + per-hit feedback), `trash.rs`, `graph_nodes.rs`, `index_runs.rs`, `repos_admin.rs`, `learning_console.rs`, `config.rs`, `memory_stores.rs`, plus `memories/edit.rs` (PATCH/restore/refresh) and `maint/{doctor,gc}.rs` — see `src/serve/routes/README.md` for per-file route list |
| `serve/jobs/` | background job model for long-running commands (`index-code`, `ingest-code`, `index`, `rebuild`, `eval`, `tune`, `bandit`, `graph-recompute`, `reembed`, `gc`, `store-sync`): `registry` (`Arc<Mutex<HashMap<JobId, Job>>>` plus one retained `watch::Sender<JobStatus>` per job, so late SSE subscriber's `borrow_and_update()` still replays terminal status; per-job `broadcast` log channel behind SSE `log` event; per-job cooperative cancel flag — `cancel(id)` marks queued job `Cancelled` outright and asks running one to stop at its next `ProgressSink::is_cancelled` boundary; `active_for(command, repo)` — liveness check behind `409 index_running`; finished jobs beyond 100 most recent are evicted on every insertion; job ids are 8 random `/dev/urandom` bytes, same entropy source as session token at shorter width), `worker` (`spawn_job` — registers job `Queued`, then on its own `tokio::spawn` task awaits single write permit FIFO for `mutating` jobs only (read-class job like `eval` never touches it), marks `Running`, runs caller's closure — typically `domains::<capability>::<cmd>::run` over `Ctx::lazy`, job's own connection — on `spawn_blocking`, and records terminal `Done`/`Error` status). `JobView` also carries `progress: Option<Progress>` and bounded 20-line `log_tail`, surfaced as SECOND SSE event type (`event: progress`) — `status` event payload itself is deliberately unchanged — `JobStatus` gained only terminal `Cancelled` variant, so existing client's `queued`/`running`/`done`/`error` events stay byte-identical. `events` holds three SSE payload types. Lifecycle is `Queued → Running → Done \| Error \| Cancelled`, not persisted: server restart forgets every unfinished job |
| `store/` | **SQLite chokepoint: only production module that imports `rusqlite`** (`src/errors.rs` excepted, see Binding Rule 10), calling no domain service in return (Binding Rule 10's second half), and single home of every SQL string in crate (migration DDL lives beside it in crate-root `migrations/`). `src/store.rs` re-exports `Connection` and `Transaction` so no other module names driver type. Flat files, one per table or concern; families that outgrow 300 lines become flat siblings with shared prefix (`rebuild_copy_*`, `code_graph_*`, `edges`/`edges_retrieval`) because guardrails' `src.nested` allows no new subfolder under `store/`. Core: `connection` (rusqlite open + PRAGMAs + `sqlite-vec` loader), `schema`, `migrate` (versioned + idempotent, applying `MIGRATIONS` slice declared in `migrate/list.rs`; DDL text in crate-root `migrations/`, `include_str!`-baked; `schema` + `schema_*` (declared toolu-orm schema — `registry()`, `DECLARED_TABLES`, and `#[table]` structs) + `schema_journal` (`just migration-journal` / `migration-adopt` operations, tested against shipped journal); `migrate/preflight` + `migrate/backup` are forward-compat guard and pre-upgrade `VACUUM INTO` snapshot run from `connection::open` before chain — see `docs/guides/upgrading.md`), `vector` (`vec0` insert/KNN with dim guard), `fts` (FTS5 helpers, code leg), `fts_memory` (memory FTS ladder behind one `run_memory_match` choke-point — every tier inherits same filters), `CreatedWindow` and `MemoryLinks` in `store.rs` (borrow-only row inputs caller fills in: `{since, cutoff}` window each SQL predicate takes, compared via `datetime()`, and `{files, symbols, documents}` reference targets `memory_row::insert` writes — first keeps `store/` free of `retrieval/` types, second free of `domains::graph` derivation), `embed` (`to_vec_blob`, dim helpers), `edge_fts` (FTS5 triplet index over `edges` — per-kind `src —rel→ dst` rendering, wholesale refresh-materialize in one tx, `needs_refresh` for upgrade self-heal, and two-tier strict→word-OR ladder behind `comemory edges`), `memory_meta` (`fetch_meta` — batched per-memory metadata: path/repo/kind/tags/references backing enriched `search --json` rows), `memory_row`/`code_row` (per-table mirror-row upserts; `memory_row::insert` takes its reference targets as `MemoryLinks` and is called only by `domains::memories::mirror`), `memory_list` (paginated memory listing, `--sort created|quality|accessed`), `eval_runs` / `gc_runs` / `index_runs` (v14/v15 run-history writers + readers; all three are in `rebuild`'s `COPIED_TABLES` — history is not reconstructable from markdown), `candidate_observations` / `candidate_judgments` (v20 candidate observation contract: one-transaction write, read back, purge's redact-the-passage-keep-the-row rule and retention sweep that never evicts judged observation; also in `COPIED_TABLES`, and never synced), `candidate_dataset` (bulk windowed read behind `comemory export-dataset`: all three of those tables in ONE read transaction, and deliberately never selecting `locator_json`, so display field cannot reach training record), `repo_drop` (`DELETE /api/v1/repos/{name}`: drop every code-index row and file edge for one repo label in one transaction, memories kept; post-commit derived refresh is `domains::code::repo_admin::disconnect`'s), `random_id` (shared random-hex id helper, moved out of `serve::security` so non-HTTP callers can use it), `code_ref` (version-anchor side table for explicit code references), `documents`/`document_fts` (document/chunk mirror + its BM25 leg), `sources` (SQLite mirror of `source::registry`), `simhash_scan` (bulk fingerprint scan shared by save + consolidate), `tokenizer/` (custom FTS5 identifier tokenizer: camelCase/snake_case split + FFI registration), `busy` (`is_locked` — answers "is this SQLite busy/locked?" from crate's own `Error`, so `serve::envelope` maps 503 without inspecting driver variants), plus per-table modules 2026-09 chokepoint refactor moved in from `api/`, `cli/`, `graph/`, `stats/`, `retrieval/`, retention rules and `eval/`: `edges`/`edges_retrieval`/`code_graph_nodes`/`code_graph_edges`, `retrieval_log`, `feedback`/`code_feedback`, `bandit_arms`, `query_expansions`, `indexed_files`, `index_failures`, `repo_marker`/`repo_marker_roots`, `prune_signals`/`prune_apply`, `gc_learning`, `doctor_probes`, `repos_inventory`, `stats_counts`, `trash_list`, `code_signals`, `schema_meta`, `sync_log`/`sync_state`/`sync_binding`, `code_sync` (projection reads and `code_sync:<repo>` cursor behind code-index push), and `rebuild_copy*` (ATTACH→copy→DETACH preservation unit, which owns its own lifecycle so no caller can leave database attached) |
| `domains/retrieval/unified/` | `comemory find`'s core (`unified::run_legs` is `find` minus fusion and pagination, returning each leg's own rows with their passage text and version anchors intact — what offline benchmark reads so it cannot drift from shipped ranking): three legs (`router`, `code_route`, `doc_route`) run unchanged and their *reranked* orders fuse via pre-existing `fuse::rrf_multi_weighted`, memory and code at weight 1.0 and documents at `retrieval.document_leg_weight` (declared and validated since document domain landed; read by nothing until this module). One shared `pipeline::pool_size` across every leg and ONE `pipeline::paginate` over fused list — RRF is prefix-stable, so divergent per-leg pools would let deeper page reorder shallower one. `fuse_domains` owns weighted fusion and `UnifiedHit`/`HitParts`, untagged enum carrying each domain's own `score_parts` verbatim |
| `domains/retrieval/` | retrieval capability (#171): `router` (candidates + 4-tier lexical ladder: strict → word-OR → subtoken-OR → tier-4 learned expansion from mined `query_expansions`), `doc_route` (document-search leg: BM25 over `document_fts`, chunk→parent coalesce), `scope` (`TimeScope` — `--since`/`--until`/`--as-of` created-date window plus its as-of supersede semantics — and `Filters`, `{repo, kind, scope}` bundle every leg narrows candidates by), `score` (ACT-R/Beta scoring primitives), `rerank` (five multiplicative priors over max-normalized relevance: activation × feedback × quality × supersede × rank, last being pool-median-relative `memories.rank_score` boost), `diversify` (SimHash near-dup collapse + MMR), `pipeline` (orchestration + access tracking), `fuse` (RRF: pairwise `rrf_k` + N-ary `rrf_multi`), `graph_route` (graph-expansion leg — one undirected recursive-CTE walk over `edges` from provisional top hits, fused in as third RRF list; `graph_hops = 0` or empty expansion returns provisional ranking untouched), `bundle` (context lookup with graph-prior-ranked code refs), `code_route` (code candidates: BM25 + thresholded ANN + RRF, chunk→parent coalesce), `code_rerank` (four-prior code rerank), `code_prior` (PageRank / recency / working-set affinity / feedback priors), `code_search` (`search_code_hits` — shared code-search entry point used by both `search-code` CLI and `GET|POST /api/v1/code/search`), `unified/` (`comemory find` core — see its own row), `code_ref_collect`/`code_ref_fetch`/`code_ref_status` (pinned code-reference freshness: collect memory's walked ref edges, fetch per-repo current state through `utilities::repo_root`, classify `fresh|stale|ghost|unpinned|unknown`), and — arriving with capability — six cores both adapters call (`search`, `search_code`, `context`, `find`, `suggest`, and console-only `config_retrieval`), result models and `--json`/`/api/v1` envelopes both transports serialize (`search_result`, `code_search_result`, `context_result`, plus `scope`'s `ScopeEcho` and its transport-neutral `resolve_domains` `--only` policy), and `explain` (explain strip: hit's `score_parts` as `{name, value, share, note}` rows, `share` being log-magnitude partition of multiplicative priors). Clap flags, TTY colouring and `--json` emission stay in `cli/` (writers in `cli/output/`) |
| `config/` | layered config (defaults → `file` → `env`) and `paths::Paths` (data-dir layout), `learning` section (`[tune]` grids, `[reinforce]`, `[bandit]`), `retrieval` section (`[retrieval]` knobs), `validate` (shared invariant pass run after every layer is applied), and `patch` (`patch_config_file` — one read-patch-atomically-write primitive over `config.toml`, shared by `tune --apply`, `hooks` reinforce toggle, and console's `PUT /config/retrieval` / `PUT /gc/policy` / `PATCH /memory-stores/{id}`; every writer validates would-be `Config` in memory before touching file) |
| `cli/output/` | TTY (`owo-colors`) and JSON (`serde_json`) writers, shared between subcommands. CLI presentation only, and since #178 that is literally true — `serve` imports nothing from here: `/api/v1/edges` body is built by `domains::graph::edges_result::envelope` and `comemory serve` startup banner is written by `cli::serve` from `serve::Ready` handover. Each retrieval command's `--json` envelope and rows are what both transports serialize, so they live in `domains::retrieval::{search_result, code_search_result, context_result, scope}` and explain strip in `domains::retrieval::explain`; `cli::output::{search, search_code, context}` keep `emit`/`write_tty` (#171), and `cli::output::edges` keeps only TTY writer (#178). `comemory::output` still resolves for library consumers through crate-root alias in `lib.rs` |
| `utilities/` | transport-neutral shared primitives, usable by every domain and by both delivery adapters, one concern per file: `blocking` (`spawn_blocking` bridge both delivery adapters share: run closure on blocking-thread-pool, flattening task panic into `crate::Error`, so connection lock never crosses `.await`), `context` (`Ctx` every command core takes: `Paths` + `Config` with connection that is either `Borrowed` — CLI's own, or server's shared per-request one — or `Lazy`, opened on first `Ctx::conn()` call, which is job worker's own dedicated connection; conn-free commands like `install`, `doctor`, `rebuild`, `ast`, `install-hooks` and `completions` never open one at all. cwd-dependent middles — `save --ref-*` anchoring, code rerank's working-set prior — resolve against *calling process's* cwd, which over HTTP is server's and not client's: documented behavior rather than bug), `pagination` (`Page<T>`, `PageWindow`, `PageMeta`, `page_window`, `page_meta`), `id_list` (CSV split/dedupe + two id-list parsers), `when` (`--since`/`--until`/`--as-of` values), `ref_args` (`--ref-file`/`--ref-symbol` → `References`), `embedding_input` (pure `--vector` CSV / JSON payload decoding) and `vector_stdin` (process-stdin acquisition split out of it), `error_code` (`classify`: transport-neutral `Error → (code, Class)` mapping `serve::envelope::status_and_code` and `mcp::result::into_tool_result` both derive their own status/error shape from, exhaustive match with no wildcard arm; `classify_project` is its nested arm for twenty-two project codes), `exit_code` (sysexits table `main.rs` exits with, moved into library so it is testable; project refusal's code follows its `Class`), `project_error` (`ProjectError`: platform's twenty-two project refusal codes with their messages and `details`) and `ordered_details` (`OrderedDetails`: `details` object serialized in insertion order, so project bodies match platform byte for byte), `embed`, `fetch`, `http_error`, `simhash`, `secret_scan` (curated secret rule set, `rules.toml`), `shared_text` (free-text policy for anything shared with another machine: secret-bearing text withheld through `secret_scan`, machine paths stripped), `digest` (SHA-256 hex + lowercase-hex shape check), `file_lock`, `path_containment`, `progress` (`ProgressSink`), `query_id`, `repo_root` (one repository resolver — `file:<repo>:<path>` id to contained absolute path, its `parse_id` decoder, and `--root` override map — shared by `cli`, `serve`, retrieval freshness and reference refresh since #167), `telemetry` (persisted `retrieval_log.source` / `feedback_events.{target_kind,provenance}` vocabularies), and `dated_id` (`<prefix>-<yyyymmdd>-<8hex>` shape both `query_id`'s `q-` ids and `observation_capture`'s `o-` ids are minted and validated through). Nothing here may import `cli` (`cli::output` writers included) or `serve`, and reference into capability is `shared layer dependency` failure unless `scripts/architecture-policy.json` declares it: `id_list` validates memory id through `domains::memories::id`, and `ref_args` resolves blob OIDs through `domains::code::git_utils` and produces `domains::memories::{Ref, References}` |
| `domains/` | business capabilities, one folder each, declared from `src/domains.rs`. capability owns its models, algorithms and command cores; it may depend on `store`, `config`, `errors`, `prelude`, `utilities`, and — only through directed table in `scripts/architecture-policy.json` — another capability. It may never import `cli` (including `cli::output` writers) or `serve`, and may never exist as empty scaffold (architecture gate fails one). Every core's `Request` derives `#[serde(deny_unknown_fields)]`, enforced by `tests/api__parity.rs`'s clap-introspection walk, so typo in flag or JSON field is rejected rather than silently ignored on either transport |
| `domains/architecture/` | architecture capability: component-level model of repository — `model` (versioned `{groups, components, edges}` value plus 200-component / 2000-edge / 32 KiB / 280-character ceilings), `cluster` (directory-prefix keys, renderer-safe ids, README-seeded summaries), `scaffold` (deterministic model built from `graph::query::build_code_graph` and `store::indexed_files`), `validate` (every save-time rule, checked against indexed paths so component cannot claim file index has never seen), `save` (validation then `memories::save::run` with `kind: note`, `tag: architecture` and `supersedes` link to previous model; `store::memory_row` writes row, this capability never issues SQL), `current` (newest live tagged memory and fenced JSON inside it), `check` (stale members, unmapped clusters and missing mined edges; report-only, like `doctor`), `mermaid` (deterministic `flowchart` source), and `learn` trio `prompt` / `learn` / `extract` — prompt file, one `sh -c` spawn of command CALLER passed under `tokio::time::timeout`, and model dug out of its stdout. comemory ships no agent command and reads none from disk. CLI-only (`serve::routes::meta::CLI_ONLY`); clap flags, `--format` switch and every rendered line stay in `cli/architecture.rs` and `cli/output/architecture.rs` |
| `domains/capture/` | capture capability (#174): client-side coding-session capture and explicit-save distillation against platform — `claude_code` (Claude Code JSONL → session metadata and Bash `tool_use` lines), `receipt` (redact + SHA-256 digest → Slice 3 wire receipt), `redact` + its own `rules.toml` (client rule set and versioned attestation every payload carries — deliberately DIFFERENT rule set from `domains/sync/rules.toml`, not duplicate to merge), `explicit_save` (recover `comemory save` claims, `claude-code-explicit-save`), `candidates` (batch types + `POST /v1/sessions/{id}/candidates`), `client` (`POST /v1/sessions`, `GET /v1/capture/sources`), `run` and `distill` (two use cases), and `hook` — `SessionEnd` contract on both ends: command installed into tool's settings AND payload that command is handed back on stdin. Credentials and API base come from `domains::sync` by concrete path; installing plugin BUNDLE is integrations' (#175). CLI-only (`serve::routes::meta::CLI_ONLY`), entered from `cli/capture.rs` and `cli/distill.rs`, which keep clap, stdin, rendering and exit codes |
| `domains/code/` | code capability (#167): `ast` + `ast/` (symbol extraction and AST pattern search via ast-grep, cAST chunking, compiled-pattern cache), `git_utils` (blob OID / HEAD / branch / remote lookups, hook installation helpers, and `repo_label` — main-worktree basename rule), `pattern_search` (`comemory ast`), `index_code` + `index_code/walk` and `ingest_code` (DB-write index paths), `index_runs` (history), `repos` + `repos/git_state` and `repo_admin` (repository inventory and its admin routes), `hooks` / `install_hooks` (git reindex hooks, resolved through git common directory), and `reindex_policy` (staleness/debounce decision behind lazy reindex — detached `index-code` launch stays in `cli::lazy_reindex`) |
| `domains/documents/` | document capability (#168): `document` + `document/` — pure, in-process extraction (TXT/Markdown/HTML/CSV) and size-bounded chunking, independent of store: `extract`/`html`/`delimited` (format-specific extractors), `chunk` (shared paragraph-boundary splitter), `fingerprint` (size+mtime skip check, SHA-256 identity), `writer` (per-file index writer: fingerprint skip → extract → one-transaction row replacement, which also calls `graph::doc_link` — single membership/reference derivation path); `source` + `source/` — durable `sources.toml` registry: `registry` (load/save, overlap validation, atomic durability; it takes shared `utilities::file_lock` guard over concurrent read-modify-write), `discover` (boundary/ignore-rule walk over registered root), `classify` (extension allowlist + binary sniff), `mirror` (reconciles TOML registry into SQLite's `source_roots`); and three command cores `index`, `sources` (whose `reconcile:false` mode is read-only probe `domains::integrations::setup::detect` uses) and `unindex`. All SQL stays in `store::{sources,documents,document_fts}` |
| `domains/graph/` | graph capability (#170): graph **algorithms** over `edges` relation — table's CRUD and every recursive-CTE walk live in `store::edges` / `store::edges_retrieval`, which these call. `cross_link` reference extraction (extraction only — `domains::memories::mirror` calls it and `store::memory_row` writes edges, #177), `cochange` (git-history co-change mining), `imports` (per-language import edges), `pagerank` (deterministic weighted PageRank), `materialize` (writes `rank_score` onto `code_symbols`), `memory_rank` (same PageRank over memory graph — direct memory→memory relations plus in-memory co-citation edges, hub rels excluded — written onto `memories.rank_score`), `coactivate` (commit co-activation reward: commit touching memory's referenced files reinforces it), `doc_link` (deterministic `member_of_source`/`references_document` link deriver; document-index seam `derive_after_document` writes its own edges, while memory-save seam `resolve_memory_documents` returns resolved ids for `domains::memories::mirror` to pass to store), `search_edit` (search→edit lookback feeding `auto_search_edit` provenance), `derived` (`refresh_derived_best_effort` — single post-write pass that refreshes *both* derived artifacts, `memories.rank_score` and `edge_fts` triplet index, independently best-effort; every caller is domain core running AFTER its own transaction commits — `memories::{save,delete,update}`, `maintenance::{rebuild,gc}`, `code::{index_code,repo_admin::disconnect}`, `sync::exchange::{import_write,code_import}` and `graph_recompute`), `neighbors` (one-hop undirected `imports`/`co_changed` file neighborhood query shared by `retrieval::bundle` and `GET /api/v1/graph/nodes/{id}/neighbors`), plus model and query assembly both transports share — `code_graph` (`Node`/`Edge`/`CodeGraph`/`GraphPage`), `query` (`Rel` relation vocabulary and `build_code_graph`/`build_graph_page` edge-window builders) and `nodes` (windowed-edge endpoint dedup and `build_graph`) — and four command cores `view`, `graph_nodes`, `graph_recompute` and `edges` |
| `domains/integrations/` | integrations capability (#175): `install` (+ `install/bundle`) extracts embedded `integrations/agent/**` assets into `<data_dir>/integrations/<version>/` as local marketplace — staging directory then rename, refusing user-edited bundle and symlinked destination or catalog — probes host CLI and `bash`/`git`/`jq`/`python3` hook dependencies before writing anything, and registers plugin with host's own manager. It is connection-free, so installing never creates database, and `.installed-<host>` marker is per version AND per host because extracted bundle is one shared tree: its existence alone would wrongly report every host as installed. Host names are validated strings, never clap enums. `install` also writes `<bundle>/plugins/comemory/.mcp.json` on every run (`bundle::write_mcp_manifest`) — plugin-root manifest both Claude Code and Codex read to spawn `comemory mcp`, carrying installing binary's absolute path, written temp-then-rename and deliberately outside embedded-bytes equality check so moving binary and reinstalling refreshes path without `bundle differs` refusal; `--dry-run` reports path and writes nothing. `setup` (+ `setup/`) is onboarding composition over seven stable step ids — `data-dir`, `agent-host`, `git-hooks`, `index-code`, `index-docs`, `reinforce`, `cloud-auth`, validated before anything is probed, `--skip` beating `--only`: `detect` is offline and read-only (it consults `domains::maintenance::doctor` only once `comemory.db` exists, lists sources with `reconcile: false`, and reads `domains::sync::auth_file` rather than asking platform), `plan` is pure, and `apply` is only phase that writes, recording step's error as its own `Failed` state so one failure cannot hide others. `cloud-auth` and `index-docs` are report-only — setup points at `comemory auth login` / `comemory index <path>` instead of signing in or indexing arbitrary root. Both cores are CLI-only, in `serve::routes::meta::CLI_ONLY` and `tests/api__parity.rs` exception list; clap shapes and host aliases, terminal detection, wizard, rendered summary and exit-code mapping stay in `cli` |
| `domains/learning/` | learning capability (#173): feedback, evaluation and searches over ranking blend. `feedback` core commits memory and code verdicts in one immediate transaction; failure rolls back both. `feedback_tracking` (per-memory `used`/`irrelevant` counters, routes' `explicit|implicit` `Source` word and its one mapping onto stored `manual`/`implicit` provenance, and `record_implicit_used` — deliberately taking bare `&Connection` so `domains::graph::materialize` mints co-activation reward inside transaction that also advances its cursor), `code_feedback` (same for symbols, keyed by stable `(repo, path, symbol)` identity with chunk-to-parent walk, since re-indexing recycles rowids), `telemetry` (`StatsDb`, shared `comemory.db` handle those two borrow their transactions from), `evaluation` + `evaluation/` (`golden` YAML sets plus `manual`-only feedback harvest, `metrics` recall@k/MRR/bootstrap CI, `runner` over real pipeline with tracking off, `mine` reformulation mining into `query_expansions`, `tune`/`tune_sample` deterministic or seeded-sampled grid search, `bandit`/`bandit_rng` eval-gated Thompson bandit on dependency-free SplitMix64 + Beta/Gamma sampler), `recall_status` (read-only, no confirm gate: tracked queries, verdicts, saves and still-pending recalls for repo + lower time bound, shared by `comemory recall-status`, `GET /api/v1/learning/recall-status` and MCP `recall_status` tool), **domain-aware offline benchmark** added by #208 (`candidate_identity` + `candidate_observation` — candidate observation contract #209 persists and #210 exports, specified in `docs/designs/2026-09-18-domain-aware-retrieval-benchmark.md`; `judgment` reviewed relevance targets; `benchmark_set` versioned reviewed set with its pinned `ranking` knobs and pre-declared budgets; `candidate_facts` + `benchmark_observe` identity/text/filter capture; `benchmark_runner` over `retrieval::unified::run_legs`, which has no telemetry path at all; `benchmark_metrics` candidate-pool recall apart from recall@k/MRR/nDCG@k with paired bootstrap; `benchmark_arm` + `benchmark_arm_report` + `benchmark_report` arms, verdicts and replayable artifact; `run_environment` hardware, memory and pinned corpus/index snapshot), **persisted** half of that contract added by #209 (`observation_capture` — opt-in, bounded, best-effort capture of real `comemory find` pool, armed only when operator opted in AND run may write telemetry; `judge` — typed verdicts resolved against captured observation, refusing candidate-pool recall miss, stale content version and unresolvable candidate, all-or-nothing, and always `manual` because typed verdict is human one), **reviewed dataset export** added by #210 (`dataset_export` — command core, which validates before database opens, reads one consistent snapshot and writes JSONL files and manifest; `evaluation::dataset_{record,rows,select,dedup,split,build,files,manifest}` — record shape and its `RECORD_VERSION`, fixed pipeline order, named drops and their counters, grouped seeded split over query/content graph, bucketing that puts splits before negative selection, owned-name sweep that makes withheld holdout real, and manifest with its `snapshot_digest` / `dataset_id`; specified in `docs/designs/2026-09-18-reviewed-dataset-export.md`), **shared-verdict** half added by #254 (`replica_payload` — `FeedbackEventV1`, one verdict on `replica-v1` wire with its device, provenance and canonical target; `feedback_share` — journalling shareable verdict inside verdict's own transaction, only moment code rowid still means its symbol), and cores every adapter calls — `feedback`, `eval`, `mine`, `tune`, `bandit`, CLI-only `benchmark`, `judge` and `export-dataset`, plus console-only `console` (`GET /learning/*`) and `learning_proposals`. All SQL is `store::{feedback,code_feedback,eval_runs,query_expansions,bandit_arms,retrieval_log}`; persisted vocabularies and query-id contract are `utilities::{telemetry,query_id}` (#166), which retrieval and graph consume directly |
| `domains/maintenance/` | maintenance capability (#176): operational corpus maintenance, one named file per operation — never shared `operations.rs`. Health: `doctor` + `doctor/` (`checks`, one function per health probe; `backup`, newest `comemory.db.pre-v{N}.bak` snapshot; `system`, facts half behind `GET /doctor/system`; `checks_replica`, `store health` and `replica identity` checks). It probes without ever CREATING database and falls back to read-only open when schema is newer than this binary — transport-neutral read `domains::integrations::setup::detect` consults once `comemory.db` exists (declared in `setup_runtime_dependencies`). Retention: `retention` + `retention/` (`low_value` signal/superseded rules, `orphans`, `stale_code` ghost `references_symbol` anchors) is read-only and side-effect free by construction, with command core `prune` owning confirmed `--apply`; `gc` reaps aged `.trash/` files AND purges their mirror rows (`store::memory_purge`) and learning telemetry (`store::gc_learning`) in same sweep; `gc_policy` holds retention windows. Duplicates: `consolidation` + `consolidation/` (`cluster` transitive union-find over stored SimHashes within radius, `keeper` ordering and in-cluster supersede resolution) behind read-only `consolidate` core — merge stays human `save --supersedes`. Repair: `rebuild` + `rebuild/copy` (`COPIED_TABLES`/`RECONSTRUCTABLE_TABLES` allowlist pair; ATTACH copy SQL itself is `store::rebuild_copy*`) snapshots still-live DB to `comemory.db.pre-rebuild.bak`, then replaces live content in place through SQLite's backup API (`store::replace_in_place`) while exchange gate (`domains::sync::exchange_gate`, `sync.lock`) pauses replication and `memory-save.lock` pauses markdown writers; `reembed` re-vectorizes through embed command with dimension guard and shared progress/cancel sink. Dashboards: `stats` and `overview` are read-only, must-not-create-the-db, and compose `domains::code::repos` / `domains::memories::list` rather than restating their policy. two owned report models are siblings of their algorithms — `retention_report`, `consolidation_report` — because core, CLI renderer and HTTP envelope all build same value. Upgrade: `upgrade` + `upgrade/` (`version` parse + ordering, `channel` Homebrew / `cargo install` / standalone detection from resolved `current_exe`, `release` `<releases>/latest` redirect → tag and one-asset `download` via `utilities::fetch` curl/wget — no HTTP client in crate, `COMEMORY_RELEASES_URL` is test hook, `installer` runs release's own `install.sh` pinned `--version --dir --no-modify-path [--quiet]` (`COMEMORY_DATA_DIR` exported) or `brew upgrade`, reads `installed_version` back, then `ensure_child` runs installed binary's own `sync daemon ensure --json` as child process and `verify_ensured` requires it ready on that exact version and canonical binary path (#258); `run` calls this after every non-`--check` outcome, including same-version no-op, and failure is `Error::Unavailable` → exit 69, with `Report.daemon` carrying `DaemonReport` under `--json`). `upgrade` is CLI-only: in `serve::routes::meta::CLI_ONLY` beside `serve` / `auth`, since server must never replace its own binary on request |
| `domains/memories/` | memory capability (#169): markdown record (`frontmatter`, `id`, `slug`, `references`, `prior`) and its atomic `store`, plus eight cores both adapters call — `save` (content-addressed replay contract, validation before every effect, near-dup advisory), `delete` (with `soft_delete` / `mirror_soft_delete` helpers `prune` and sync import reuse), `list`, `show`, `update` (+ `mirror_record`, one re-mirror path), `restore`, `trash`, `refresh_refs`, and `nav` (memory's title and markdown path), plus `mirror` — single seam every writer here, in `maintenance::rebuild` and in sync import goes through, which derives body's graph links and hands them to `store::memory_row` as row data (#177). SQLite stays in `store`; clap flags, process I/O and inline cloud push stay in `cli` |
| `domains/sync/` | sync capability (#172): org-scoped credential and login that mints it — `auth_file` (auth.json schema v2; v1 refused; `load_usable` is read-only probe `domains::integrations::setup::detect` uses), `cloud` + `cloud/` (`api_url` — `--api-url` > `COMEMORY_API` > `https://api.comemory.io` — and `device`, RFC 8628 code/token poll plus `POST /v1/device/mint-org-key`; device-code grant is only approval channel, so workspace travels *in key* and mint missing `organizationId`/`workspaceId` is refused rather than written), and `login` (three `comemory auth` sequences with progress writer injected and nothing rendered); wire protocol's two halves — `client` (reqwest+rustls; Authorization only, plus `ws_ticket`/`channel_url` for workspace channel), `client_code`, and `exchange` + `exchange/` (*server* half `serve` routes call: wire models plus `changes` / `manifest` / `import` / code-import cores); exchange client — `drain` + `drain/` (#255: one entry point every caller drains through — session open and protocol negotiation keyed by `(api_url, workspace)`, `replica-v1` pass and legacy pass under one loop, budget and network state, holds, compacting replay, rebootstrap, verify and `exchange` status block); directions of travel — `push`/`pull` (legacy pass's batch and page), `verify`, `initial` (drain at login, then legacy code push), `manual` (what one `comemory sync` run does), `push_on_save` (one push-only pass after local write, skipped when pass holds `sync.lock`, called from CLI seam only so HTTP write starts no outward sync), and `code` + `code_plan` + `client_code` (code-index push: every indexed repo's snippet-free projection — paths, blob OIDs, symbol names and line ranges, `imports`/`co_changed` edges — diffed by blob OID against workspace's `GET /v1/sync/code/manifest` and posted to `POST /v1/sync/code/import`); filters and resident loop — `redact`/`rules.toml` (curated secret scan before push is enqueued), `skip_repos` (ONE client-side filter left — empty `repo` label no longer withholds anything, since label comes from cwd's git repo and made sync eligibility depend on which directory `save` ran in), `watch` (raw platform-channel follower `channel_loop`/`Frame` — harness-switch fallback `cli::watch` uses directly, and what coordinator's own `daemon::channel` wraps), `auth_barrier` (durable logout barrier, `auth.disabled`: every credential read answers logged out while it stands, cleared only by explicit login), and `daemon` + `daemon/` + `daemon_unit` + `daemon_templates` (#257: **required**, always-resident per-directory coordinator every ordinary command's preflight verifies and repairs — see [`domains/sync/daemon/README.md`](src/domains/sync/daemon/README.md) for its control-socket protocol, lifecycle and supervisor pieces; `daemon_unit` is now just `launchctl`/`systemctl` shell-out `daemon::supervisor` shares, and `daemon_templates` per-directory plist/unit text); and `memory_store` + `memory_store/` — different kind of sync: Git synchronization of data directory's own work tree, with its own `store-sync` job and log contract. `replica/` is `replica-v1` journal protocol and its engine halves for every entity kind — memories, code generations, document revisions and, since #254, feedback and activity events (`activity_payload`, `event_capture`, `event_accept`, `validate_events`), each counted once per event whatever operation id carries it. Clap flags, prompts and every rendered line stay in `cli`; all SQL stays in `store::{sync_log,sync_state,sync_binding,code_sync,sync_exchange,sync_policy_snapshot,replica_*}` |
| `errors.rs` | `thiserror`-derived `Error` enum and `Result<T>` alias |
| `prelude.rs` | crate-internal prelude (`Error`, `Result`, common imports) |
| `lib.rs` / `main.rs` | library surface (carries `extern crate self as comemory;`, see Testing) + binary entry that parses `Cli` and calls `cli::run` |

## Environment Variables

Values are layered: defaults (`Config::defaults`) → optional config file →
environment (`Config::with_env`, in `src/config/env.rs`).

| Variable | Purpose | Default |
|----------|---------|---------|
| `COMEMORY_DATA_DIR` | Root data directory (`memories/` + `comemory.db`) | `~/.comemory` |
| `COMEMORY_INDEXING_AUTO_REINDEX` | `lazy` \| `hook` \| `off` — automatic code-index refresh. `lazy` (wired in `src/cli/lazy_reindex.rs`): `search-code`/`context` spawn detached, non-blocking `index-code` when repo HEAD moved since last index, then search current index immediately; `hook` relies on installed git hooks; `off` is manual-only | `lazy` |
| `COMEMORY_RETRIEVAL_TOP_K` | Number of results returned by hybrid router (also default page size for `search` / `search-code` / `context` when `--k`/`--limit` is omitted) | `12` |
| `COMEMORY_RETRIEVAL_MAX_PAGE_WINDOW` | Maximum depth pagination can reach into ranked result list. `search` / `search-code` / `context` fetch candidate pool sized `clamp(offset + k + k, CANDIDATE_POOL, max_page_window)`, run full fuse → rerank → diversify pipeline over it, then slice `[offset, offset+k]`; `has_more` is forced false once this ceiling is hit (deeper results require refining query). Validated `> 0`. | `200` |
| `COMEMORY_RETRIEVAL_MEMORY_THRESHOLD` | Minimum cosine similarity for memory table | `0.55` |
| `COMEMORY_RETRIEVAL_CODE_THRESHOLD` | Minimum cosine similarity for code table (ANN leg of `search-code`, range `[0.0, 1.0]`) | `0.50` |
| `COMEMORY_RETRIEVAL_RRF_K` | RRF fusion constant for hybrid scoring | `60.0` |
| `COMEMORY_RETRIEVAL_GRAPH_HOPS` | Maximum hop depth of graph-expansion leg — `edges` walk seeded from provisional top hits of memory `search`. Validated `≤ 4`; `0` disables leg and retrieval takes legacy two-leg path unchanged | `2` |
| `COMEMORY_RETRIEVAL_GRAPH_SEEDS` | How many provisional top hits seed that walk. Validated `≥ 1` | `8` |
| `COMEMORY_RETRIEVAL_BM25_WEIGHTS` | `"body,tags"` BM25 column weights for `memory_fts` (both finite ≥ 0, at least one > 0) | `1.0,3.0` |
| `COMEMORY_RETRIEVAL_CODE_BM25_WEIGHTS` | `"symbol,snippet,path_tokens"` BM25 column weights for `code_fts` (all finite ≥ 0, at least one > 0) | `2.0,1.0,1.5` |
| `COMEMORY_RETRIEVAL_DOCUMENT_LEG_WEIGHT` | Weighted-RRF contribution of document leg, relative to memory and code legs (both fixed at `1.0`), in default `search` order. Finite, in `(0.0, 10.0]`. Deliberately absent from `[tune]` grids — `tune`/`bandit` pin memory-only scope, so memory-only metric would drive this cross-domain knob toward zero | `0.5` |
| `COMEMORY_INDEXING_MAX_FILE_BYTES` | Ceiling (bytes) above which candidate document file is recorded `too_large` and skipped by `comemory index` document writer rather than extracted — plain-text formats past this size are logs, not documents. Must be > 0 | `16777216` (16 MiB) |
| `COMEMORY_LEARNING_RETENTION_DAYS` | `comemory gc` retention window (days) for raw `retrieval_log` + `feedback_events` rows, and for UNJUDGED captured candidate observations; aggregated `feedback` counters, mined `query_expansions` and any observation carrying reviewed judgment never expire | `90` |
| `COMEMORY_OBSERVATIONS_ENABLED` | Opt in to candidate observation capture: `comemory find` records its whole candidate pool — each candidate's stable identity, content version and bounded passage — and prints `o-<yyyymmdd>-<8hex>` handle `comemory judge` takes. Off by default, because capture stores passage snapshots. Capture requires run that may write telemetry, so read-only `comemory serve` and `COMEMORY_DISABLE_ACCESS_TRACKING` both suppress it, and capture that fails never fails search (`[observations] enabled`) | `false` |
| `COMEMORY_OBSERVATIONS_MAX_TEXT_BYTES` | Per-candidate passage bound in bytes; digest always covers FULL passage before bounding. Validated `> 0` (`[observations] max_text_bytes`) | `4096` |
| `COMEMORY_OBSERVATIONS_MAX_CANDIDATES` | Ceiling on candidates persisted per captured query; raised when it would cut into returned page, and cut pool is recorded `truncated` so consumer knows not to compute pool recall from it. Validated `> 0` (`[observations] max_candidates`) | `100` |
| `COMEMORY_TUNE_MIN_GOLDEN` | Test hook lowering `comemory tune` / `comemory bandit`'s minimum-golden-pairs floor; not tuning knob | `10` |
| `COMEMORY_REINFORCE_SEARCH_EDIT_DAYS` | Lookback days for search→edit auto-reinforcement provenance (`auto_search_edit`). Validated `≥ 1` | `7` |
| `COMEMORY_DISABLE_ACCESS_TRACKING` | Test hook (truthy) disabling `search` / `context` access tracking + `retrieval_log` writes for one run, so stability harness can drive binary repeatedly without each query mutating `access_count` / `last_accessed`. Only head page is bumped since #201, so pure deep-paging loop no longer needs this hook; what it still buys harness is frozen *first* page, whose rows repeated query does re-reinforce. Not user knob | `false` |
| `COMEMORY_SYNC_PUSH_ON_SAVE` | Push outbox inline after `save` / `delete` (`[sync] push_on_save`). This is what lets continuous sync stop depending on resident process | `true` |
| `COMEMORY_SYNC_CODE_INDEX` | Push code index of every indexed repo alongside memories (`[sync] code_index`): paths, blob OIDs, symbol names and line ranges, `imports` / `co_changed` edges — never source text. On by default so console's graph fills in right after `auth login`; `skip_repos` still withholds repo | `true` |
| `COMEMORY_SYNC_PUSH_ON_SAVE_TIMEOUT` | Budget for that inline push only (`[sync] push_on_save_timeout`), far below 30s every other platform call takes: save must return on captive-portal network. Validated `> 0` — disable hook with `push_on_save = false` rather than zero budget | `2s` |
| `COMEMORY_GIT_AUTO_SYNC` | `true`/`1` to enable best-effort git commit + push after save. Also file key: `[git] auto_sync` / `[git] remote` in `config.toml` (written by `PATCH /api/v1/memory-stores/default`); env wins over file | `false` |
| `COMEMORY_EMBED_HINT` | Free-form identifier of embedder you used (e.g. `ollama:nomic-embed-text`). Surfaced by `comemory doctor`; never consumed as switch. | unset |
| `COMEMORY_EMBED_CMD` | Embed command used by `comemory serve`'s `POST /api/v1/doctor/reembed` (re-vectorize memories/code server-side). Run as `sh -c <cmd>`; reads text on stdin, must emit `{"embedding":[..]}` on stdout. `serve --embed-cmd` overrides it. Unset → reembed answers `503 embedder_unavailable`; lexical search always works. | unset |
| `COMEMORY_RANK_DECAY` | ACT-R decay exponent `d` in `ln(n) − d·ln(days+1)`. Must be ≥ 0. Higher → older memories decay faster. | `0.5` |
| `COMEMORY_RANK_PRIOR_CLAMP` | `"lo,hi"` bounds applied to activation, feedback, quality, and PageRank boost multipliers (fixed `0.2` supersede penalty intentionally bypasses clamp). Both finite; lo > 0, lo ≤ hi. | `0.5,2.0` |
| `COMEMORY_RANK_MMR_LAMBDA` | MMR relevance-vs-diversity trade-off in `[0.0, 1.0]`. `1.0` = pure relevance; `0.0` = pure diversity. | `0.7` |
| `COMEMORY_RANK_NEAR_DUP_HAMMING` | SimHash Hamming radius for near-dup detection (save-time advisory + diversify collapse). Must be ≤ 64 (SimHash is 64-bit). | `8` |
| `COMEMORY_PRUNE_MIN_ACTIVATION` | Activation floor (ACT-R scale) below which memory is prune-eligible. | `-2.0` |
| `COMEMORY_PRUNE_MIN_FEEDBACK` | Beta-feedback ceiling (range `[0.0, 1.0]`) at or below which memory is prune-eligible. | `0.25` |
| `COMEMORY_PRUNE_BELOW_QUALITY` | Quality threshold (1..=5); memories at or below this value are prune candidates (used together with activation + feedback floors). | `2` |
| `COMEMORY_PRUNE_SUPERSEDED_GRACE_DAYS` | Grace window (days) before superseded-and-never-accessed memory becomes prune-eligible; protects freshly-rebuilt DBs whose supersede edges all carry rebuild-time timestamps. | `7` |
| `COMEMORY_RELEASES_URL` | Test hook: release base `comemory upgrade` and `install.sh` resolve `latest` and download assets from (`<base>/latest`, `<base>/download/<tag>/<asset>`); suite points it at loopback stand-in (`tests/common/release_server.rs`). Not user knob. | `https://github.com/Falconiere/comemory/releases` |
| `COMEMORY_API` | Platform API base for `comemory auth` (trailing slash stripped). Overridden by `auth login|status --api-url`. | `https://api.comemory.io` |
| `COMEMORY_API_KEY` | Optional override of `auth.json` org-key secret (same idea as CI without writing file). | unset |
| `COMEMORY_SKIP_MIGRATION_BACKUP` | Truthy (`1`/`true`) skips pre-migration `VACUUM INTO` snapshot (`comemory.db.pre-v{N}.bak`) that `store::migrate::preflight` otherwise takes before ANY pending schema migration — failed snapshot only refuses upgrade when pending migration is destructive, and merely warns otherwise. See `docs/guides/upgrading.md`. | `false` |

`[reinforce] enabled` (default `true`) is file-only, with no env override:
it is on/off bit behind `comemory hooks --enable|--disable
search-edit-reinforcement`. `COMEMORY_REINFORCE_SEARCH_EDIT_DAYS` still sets
lookback window, but window cannot express "off" (it is validated
`≥ 1`), which is why flag is separate boolean rather than magic `0`.

`[tune]` knobs are file-only — set them in `config.toml`; they have no
env override. Six grids (`tune.rrf_k_grid`, `tune.decay_grid`,
`tune.mmr_lambda_grid`, `tune.bm25_grid`, `tune.graph_hops_grid`,
`tune.graph_seeds_grid`) define search space — 729 candidates at 
defaults — and `tune.samples` (default `64`) caps how many of them run
scores by drawing that many distinct candidates from pools with
derived-or-`--seed`-pinned PRNG; `tune.samples = 0` restores exhaustive
cartesian sweep. `comemory bandit` ignores `tune.samples` — its arms stay 
full grid.

memory and code vector dims (1024 and 768) are baked into 
`memory_vec` / `code_vec` vec0 DDL (`migrations/0002_v2_tables.sql`)
at migration time and are not env-configurable: divergent env value
would silently disagree with vtab and surface as `VecDimMismatch`
at first insert. Change literal in DDL if you need different
dim.

CLI flags `--data-dir` and `--json` are global and can appear before or
after subcommand.

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
   set, parse it into `Vec<f32>` and run `store::embed::dim_guard`
   against `schema_meta` so mismatched embedder fails fast with
   `Error::VecDimMismatch`. With neither flag, save is lexical-only —
   no `memory_vec` row is written.
2a. **Prior lookup** (`MemoryStore::prior`, `src/domains/memories/prior.rs`), run
   once vector is parsed and data dir exists but BEFORE 
   database is opened — so ahead of step 2's dim guard: what
   `memories/{id}-*.md` — else `.trash/{id}-*.md` — already holds for 
   content-derived id. prior whose `content_hash` differs is 32-bit
   collision → `Error::IdCollision` (HTTP `409 id_collision`, CLI exit 65)
   with nothing written and no DB touched. matching prior contributes its `created`, so 
   replay never re-stamps markdown, and makes response
   `created: false`. same lookup backs `domains::sync::exchange::import_state`'s
   pulled-record collision rule.
2b. **Near-duplicate check** (best-effort, advisory): scan live `memories`
   rows for SimHash Hamming distance within `NEAR_DUP_HAMMING`. If 
   near-duplicate is found its id is recorded as `duplicate_of`. save
   always proceeds; caller decides whether to re-save with
   `--supersedes <id>`. TTY
   mode prints `warning: similar memory <id> exists` to stderr;
   `--json` mode includes `"duplicate_of": "<id>"` in output object.
   Self-matches (re-save of same body, same content-hash-derived id)
   are filtered out.
3. Atomic stage: write `memories/.{id}.tmp`, then `fs::rename` to
   `memories/{id}-{slug}.md`. On failure between stage and rename, tmp
   file is removed.
4. Single `store` transaction:
   - upsert `memories` row (frontmatter + body + simhash)
   - upsert `memory_fts` row (FTS5)
   - upsert `memory_vec` row (`vec0`) when vector was supplied
   - frontmatter `relations.{supersedes,conflicts_with,derived_from}` ids
     (`supersedes` is populated by `--supersedes`; others come from
     hand-edited markdown) are materialized as memory→memory `edges` rows.
     Targets may dangle — every consumer (rerank's supersede penalty,
     prune's superseded-rule, `supersedes_chain`) joins on live `memories`
     rows. `comemory rebuild` re-materializes these edges from markdown.
   - `cross_link::extract_refs` walks body for backtick-fenced
     `<repo>:<path>` / `<repo>:<path>:<symbol>` mentions; `memories::mirror`
     hands result to `store::memory_row`, which writes 
     `ReferencesFile` / `ReferencesSymbol` rows into `edges`. Missing
     `code_symbols` rows are tolerated — `comemory index-code` fills them
     in later. URLs (`://`, `git@host:`) and path expressions behind bare
     scheme (`file:/tmp/x.db`, `sqlite:./x.db`, `file:../x.db` — any
     captured path starting with `/`, `./` or `../`) are never references;
     migration `0018` deletes ones older binaries minted.
5. Best-effort git auto-sync via `git_utils`, only when
   `COMEMORY_GIT_AUTO_SYNC` is enabled.
5a. **Inline cloud push** (`cli::save` only, not `domains::memories::save`): when
   `[sync] push_on_save` is on and `auth.json` is usable, run one push-only
   drain pass (`domains::sync::drain`) under `push_on_save_timeout`, skipping
   it when pass already holds `sync.lock`. When that push does not fully
   drain outbox, sends best-effort wake to required coordinator
   (#257) so entry does not wait for its next tick. Never fails 
   save — offline, unauthenticated and refused all leave entry in 
   outbox for that wake, or next `sync` / `watch` / coordinator cycle
   either way. It lives in `cli::` because
   `domains::memories::save` also runs inside `comemory serve`, where
   `reqwest::blocking`
   panics on drop and pushing tenant's memories outward would be wrong.
6. Answer `{id, path, created, duplicate_of?, warnings?}`. `created: false`
   is idempotent replay (same body re-saved, metadata overwritten
   last-writer-wins, or trashed id revived); TTY prints `updated <id>`
   instead of `saved <id>`. Stated contract, pinned by
   `tests/cli__save_3.rs` and `tests/serve__routes__memories__write.rs`,
   documented in `docs/guides/http-api.md` § Save contract.

If SQLite mirror transaction fails, markdown file is **kept** (it
was already written as source of truth) and error wraps 
markdown path with hint to run `comemory rebuild`. `rebuild` fully
reconstructs memory layer — rows, `memory_fts`, and edges — from
`memories/*.md`, and (snapshotting still-live `comemory.db` to
`comemory.db.pre-rebuild.bak` first) preserves everything markdown cannot
rebuild — code index, document index, and learning-loop
tables — by copying it from pre-rebuild database rather than
regenerating it. Two things it does **not** do: repopulate `memory_vec`
rows (BYO-vector contract means only caller's embedder can do
that — re-run `comemory save` / `ingest-code`), and re-walk indexed repos
(code/document index is carried across, not rebuilt from source). See
README "BYO-Vector workflow" section, `scripts/comemory-embed.sh` for
recommended caller pattern, and `docs/guides/upgrading.md` for 
schema-migration snapshot `rebuild` shares its mechanism with.

## Testing

- Runner: `cargo nextest run --all-features` (alias `just test`).
- Match `.github/workflows/test.yml`'s Rust 1.95 toolchain when running 
  quality gates (`RUSTUP_TOOLCHAIN=1.95.0 bash scripts/check-all.sh`); Clippy
  lint inventories differ between compiler versions.
- Linux CI places coverage-run temporary fixtures on isolated 2 GiB `tmpfs`
  mount to avoid block-device contention from thousands of SQLite commits.
  These remain real files and databases across process restarts. Nextest starts
  four large-backlog exchange tests first; test counts and assertions stay
  unchanged, and normal debug test profile retains its runtime checks.
- **Test code never lives in production file.** No `#[cfg(test)] mod tests { ... }`
  body in any `src/` file, ever.

### Where a test goes

Two homes, and one rule decides between them. Applied in order, first match
wins:

1. **It drives real `comemory` binary** (`assert_cmd`) -> crate-root
   `tests/`. subprocess consumer is most real consumer there is, and
   `cargo_bin` resolution is only reliable for integration target.
2. **It owns `insta` snapshot** -> crate-root `tests/`. insta derives its
   snapshot directory from test file's location; `tests/snapshots/` is 
   one reviewed home.
3. **It exercises CLI surface** (`tests/cli*.rs`) -> crate-root `tests/`.
   CLI is one public surface; its suite stays in one place.
   Per-command contracts live in `tests/cli__<cmd>.rs`. Multi-command
   user journeys live in `tests/cli_scenario_*.rs` and share
   `tests/common/cli_bin.rs`; each journey has `/api/v1` twin in
   `tests/serve_scenario_*.rs` over real `comemory serve`, sharing
   `tests/common/serve_bin.rs`. `docs/scenarios/` is human-readable
   test plan (one file per subcommand, every flag, its HTTP route, which
   test covers it); `tests/cli_scenario_catalog.rs` walks built
   `Cli::command()` plus live `GET /api/v1/commands` and fails when 
   subcommand, flag, positional, or route is missing from its file, when 
   flag has no scenario citing test, or when cited `tests/…rs::fn`
   does not exist.
4. **Otherwise** -> colocated, in sibling `tests/` folder beside module
   under test:

       src/store/migrate.rs          production module
       src/store/tests/migrate.rs    its tests
       src/utilities/simhash.rs      a shared primitive
       src/utilities/tests/simhash.rs  its tests

   production file names its test files with one-line include, which makes
   it index of its own suite:

       #[cfg(test)]
       #[path = "tests/migrate.rs"]
       mod tests;

Keep each `tests/` tree **flat**. suite that outgrows one file splits into
`<name>_2.rs` (or `<name>_v4.rs` for version-scoped suite) beside it, and 
production file gains second include (`mod tests_2;`). Tests are exempt from
300-line ceiling, so splitting is readability choice, not obligation.

Measured today (2026-09-02): 194 colocated `src/**/tests/*.rs` files, 106
crate-root `tests/*.rs` surfaces (61 CLI, 8 HTTP journeys, 80 `assert_cmd`, 3 `insta`).

### Why crate-root `tests/` cannot be deleted

Rules 1 and 2 above are not stylistic preferences, and question "why not
colocate *everything* and drop `tests/`?" recurs. It was measured on
2026-08-07; answer is that three separate things anchor directory.

**1. `CARGO_BIN_EXE_comemory` (binding constraint).** `assert_cmd`'s
`Command::cargo_bin("comemory")` reads `CARGO_BIN_EXE_comemory` env var,
which Cargo publishes **only to integration-test and bench targets**. From 
lib unittest it is unset and assert_cmd panics outright — its own help text
says *"if this is running within unit test, move it to integration test
to gain access to `CARGO_BIN_EXE_comemory`."*

There is trap here worth knowing about. If `target/debug/comemory` already
exists, `cargo-nextest` sets variable at runtime and colocated
`assert_cmd` test passes — so on warm target dir constraint looks
absent. It is not. Delete binary, or run `cargo nextest run --lib`, and
same test panics. Verified both ways, and separately on scratch crate
with no `tests/` directory at all, where `cargo nextest run` never built 
binary.

consequence of removing last integration target is therefore worse
than build error: nothing would make Cargo rebuild binary during test
run, so 32 `assert_cmd` surfaces would silently exercise **whatever stale
binary was left in `target/`** and report green against source you had already
changed. false green is worse failure mode than red build, which is why
this is rule rather than preference.

If directory is ever collapsed anyway, cheapest way to keep 
guarantee is to leave exactly one integration target whose only job is to
force binary build — not to drop to `cargo build --bin comemory && cargo
nextest run`, which fixes only sanctioned entry points and leaves bare
`cargo nextest run` silently stale.

**2. insta snapshot resolution.** insta derives its snapshot directory from
test file's own location, so colocating 4 snapshot owners would
scatter `snapshots/` directories under `src/` — which `folder-tree`
guardrail rejects (`src.nested` allows only `tests` and
`proptest-regressions`). It would need either new allowlist entry or 
redirected `Settings::set_snapshot_path`, and `tests/snapshots/` stays one
reviewed home instead.

**3. Fixture and data files.** `tests/common/` holds single copy of every
shared fixture (D9), and `tests/golden/`, `tests/ast/fixtures/` and
`tests/common/fixtures/` hold data addressed through
`concat!(env!("CARGO_MANIFEST_DIR"), "/tests/...")`. These are movable
part — `tests/common/*.rs` could become `src/test_common/*.rs` and data
could move to top-level `fixtures/` — but only in world where constraint 1
is already solved, because `#[cfg(test)]` items in `src/` are invisible to 
integration-test binary.

### Conventions inside a test file

- First line, always:

      #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::float_cmp, clippy::too_many_lines)]

  failed assertion is how test reports; unwrapping is reporting
  mechanism, not bug. This is toolu Rust template's own header plus 
  three lints that only ever fire in test code here.
- Import through crate name — `use comemory::store::migrate::{...};` — in
  **both** homes. `src/lib.rs` carries `extern crate self as comemory;` so 
  test file reads identically wherever it lives, and tests keep exercising 
  public surface rather than ossifying internals. Reach for `use super::*;` only
  when test genuinely needs private item.
- Shared fixtures live in `tests/common/` and are reached from colocated test
  through bridge module: `use crate::test_common::git_repo;`. Crate-root
  tests keep including them directly with `#[path = "common/git_repo.rs"]`.
  There is exactly one copy of each fixture. bridge itself is
  `src/test_common.rs` — `#[cfg(test)]`-only module declared from
  `src/lib.rs` that re-`#[path]`s eight fixtures colocated tests 
  use; it is migration's one sanctioned `#![allow(dead_code)]` under
  `src/`, exempted by name in `no-allow-attribute.yml`.
- Data files (`tests/golden/`, `tests/common/fixtures/`, `tests/ast/fixtures/`)
  stay at crate root and are addressed with
  `concat!(env!("CARGO_MANIFEST_DIR"), "/...")`, which is stable no matter where
  test file lives. Never `../../..` relative path.
- Real data, real integration paths. No mock-data tests.

`.config/nextest.toml` serializes two groups: `embedder`
(`embedder|memory_index|code_index`), so fastembed model download cannot
race, and `config::env::tests*`, because those tests mutate process-global env
vars and colocation put them in crate's single lib-unittest binary
alongside every other colocated test (`binary(...)` filter no longer
isolates them; name filter does).

## Quality Gates

CI runs these gates on ready-for-review PRs targeting `main` (and on later
commits to those PRs). Draft PRs wait for `ready_for_review`. Pushes to `main`
and release tags do not rerun quality suite; release tags still perform
release-specific preflight and artifact checks.

`bash scripts/check-all.sh` is umbrella gate. It runs, in order:

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

Retired in toolu-conventions migration, folded into two gates above:
`test-placement-check`, `no-bypass-check`, `module-size-check`,
`tests-mirror-check`.

Gate ownership — one rule, one enforcer. `guardrails.config.json` is 
single declaration of every structural ceiling (file size, function size, 
nested-folder allowlist, required per-folder READMEs); nothing else declares
competing number.

| Rule | Owner |
| --- | --- |
| `rustfmt` formatting | `rustfmt.toml`, `scripts/fmt-check.sh` |
| Type/borrow-check | `cargo check`, `scripts/type-check.sh` |
| unwrap/expect/panic/todo/unimplemented/print_*/`too_many_lines`/pedantic | `Cargo.toml [lints]` + `clippy.toml`, `scripts/lint-check.sh` |
| File size (300 code lines), function size (100 lines) | `guardrails.config.json` (`fileSize.max`, `functionSize.max`), `file-size` guardrails check and `clippy::too_many_lines = "deny"` |
| No `mod.rs` barrels | `guardrails.config.json` (`barrelNames`), `no-barrels` guardrails check |
| Folder tree shape (which subfolders each module may have) | `guardrails.config.json` (`src.nested`), `folder-tree` guardrails check |
| `snake_case` filenames | `guardrails.config.json` (`filenameCase`), `filename-case` guardrails check |
| Required per-folder `README.md` | `guardrails.config.json` (`src.requireReadme`, `src.nested "x/*"`), `folder-readmes` guardrails check |
| No inline `#[cfg(test)] mod tests { ... }` | `scripts/guardrails/patterns/rust/no-inline-test-module.yml` |
| No direct `std::env::var` outside `config/`/`tests/` | `scripts/guardrails/patterns/rust/no-direct-env-var.yml` |
| No `unsafe` without `// SAFETY:` comment | `scripts/guardrails/patterns/rust/no-unsafe-without-safety.yml` (project-local, D1) |
| No `#[allow(...)]` in production code | `scripts/guardrails/patterns/rust/no-allow-attribute.yml` (project-local, D6) |
| Committed secrets, shadow configs (`lefthook.yaml` vs `.yml`) | `guardrails.config.json` (`secrets`, `shadowConfigs`), `secrets`/`shadow-configs` guardrails checks |
| Typos | `typos.toml`, `scripts/typos-check.sh` |
| `docs/cli-reference.md` drift | `scripts/cli-docs-check.sh` vs real `--help` output |
| Shipped migration SQL is immutable | `scripts/migration-check.sh` (git-tag-dependent; compares each `migrations/*.sql` against its first release tag, accepting pre-v0.29 location under `src/store/` for same basename) |
| Duplication ratchet | `scripts/dup-check.sh` against `dup-baseline.txt`; it **enforces** `similarity-rs` build named by its `SIMILARITY_RS_VERSION` constant, hard-failing on any other version (see `docs/dup-debt.md`) |
| rusqlite confined to `src/store/` | `store-leak-baseline.txt`, `scripts/store-chokepoint-check.sh` |

Three further gates need separately installed tool, so they are deliberately
outside `check-all.sh`'s `GATES` array — and all three run both in `just qa` and
as their own steps in `.github/workflows/test.yml`:

| Gate | Tool, and constant that pins it |
| --- | --- |
| `scripts/deny-check.sh` (licence + advisory policy) | `cargo-deny`, `CARGO_DENY_VERSION` |
| `scripts/dup-check.sh` (duplication ratchet) | `similarity-rs`, `SIMILARITY_RS_VERSION` |
| `scripts/machete-check.sh` (unused dependencies) | `cargo-machete`, `CARGO_MACHETE_VERSION` |

Each constant lives on one line in its own script and is single source of
truth: workflow *derives* version it installs from that line instead of
repeating it, so workflow and gate cannot drift, and bumping 
constant is only edit version bump needs. two cases are handled differently, and difference
matters: **version mismatch** is hard failure everywhere, CI and local
alike, because verdict from another build is not verdict CI will reach;
**missing tool** fails in CI but locally prints loud `[warn]` saying 
gate did not run, names pinned install command, and **exits 0**. So green
`just qa` on machine without tool means "this gate was skipped, loudly",
not "this gate passed" — CI is run that decides.

`just qa` also runs `scripts/test-install-libc.sh` — `install.sh`'s libc gate
on real distro images. It needs reachable docker daemon and network; missing
daemon, or case whose image or packages cannot be fetched, follows same
missing-tool rule (warn locally, fail under `$CI`). It is NOT `test.yml` step.

Two of them assert that their tool ingested tree, because gate
that reports success while checking nothing is this repo's recurring failure
mode: `dup-check` parses `similarity-rs`'s `Checking N files` line and fails
when `N` differs from file count it handed over, and `machete-check` runs
`cargo-machete` over copy of this crate's own `Cargo.toml` first and fails
unless that canary reports unused dependencies — tool exits 0 with 
success message when handed directory containing no manifest at all.
`deny-check` asserts that cargo-deny read this repo's `deny.toml`, gathered 
crate count floored against `Cargo.lock`, fetched advisory database, and
reported all four of its checks.

`scripts/test-run.sh` runs nextest suite. task is not "done" until
`scripts/check-all.sh` exits 0.

## Distribution

- `curl … https://github.com/Falconiere/comemory/releases/latest/download/install.sh | sh`
  — repo-root `install.sh`, hand-written (POSIX sh, ≤ 300 code lines like
  every other file), attached to every release by `release-finalize.yml`
  (which also smoke-runs it against fresh release on Linux runner).
  Platform detection plus musl refusal before any download (glibc read from
  `getconf GNU_LIBC_VERSION`, musl from `/bin/sh`'s ELF interpreter, never
  from mere `/lib/ld-musl-*` presence; glibc version is left to `--version`
  run, since only each release's binary knows `GLIBC_` it needs —
  `scripts/test-install-libc.sh` runs gate on real distro images under
  `just qa`), `latest` via release redirect, SHA-256 sidecar
  verification (never skipped), `--version` run of downloaded binary
  before atomic rename, in-place replacement of `comemory` already on
  `PATH`, once-only rc-file `PATH` line. It finishes by running just-placed
  binary's own `sync daemon ensure` and does not report success until that
  coordinator answers ready on that exact file; mismatched or unreachable
  one rolls swap back (or, on first install, keeps new file) and
  exits **69** (#258). Its `--version` / `--dir` / `--no-modify-path` /
  `--quiet` flags are contract: `comemory upgrade` runs it with exactly
  those, plus `COMEMORY_DATA_DIR` so it ensures right data directory's
  daemon. command with its hardening flags is spelled out in README §
  Install. `scripts/dev-install.sh` does same after `cargo install --path
  .` — build, install, `sync daemon ensure`, `completions --install` — and is
  recommended way to finish source install rather than bare `cargo
  install --path .`, which places binary and starts nothing (unmanaged
  placement; next ordinary command's preflight repairs daemon, or run
  `comemory sync daemon ensure` by hand).
- `https://get.comemory.io/pkg/comemory/install` — 302 served by 
  `comemory-prod` Cloudflare Worker on comemory.io platform; it still
  points at cargo-dist's `comemory-installer.sh` until that worker is
  repointed at `install.sh` asset above.
- `comemory-installer.sh` — cargo-dist generated installer, still
  published on every release (`installers = ["shell", …]`); no pinning, no
  in-place upgrade, skips checksum on stock macOS.
- `cargo install --path .` (build from local checkout; not published to
  crates.io).
- `brew install Falconiere/tap/comemory` (Homebrew tap
  `Falconiere/homebrew-tap`, published by `cargo-dist`).
- Prebuilt tarballs for `aarch64-apple-darwin`,
  `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu` attached to
  [GitHub Releases](https://github.com/Falconiere/comemory/releases).

`cargo-dist` is configured in `[workspace.metadata.dist]` in `Cargo.toml`.
PRs get dry-run plan; only version tags (e.g. `vX.Y.Z`) publish artifacts.

Releases are driven by **release-plz** bot (`.github/workflows/release-plz.yml`,
config `release-plz.toml`): push to `main` opens/updates "release PR" that bumps
version + rewrites `CHANGELOG.md` from conventional commits — but only when at
least one commit since last tag would render changelog line (`release_commits`;
`ci:`/`docs:`/`chore:`-only push opens nothing); merging it pushes 
`vX.Y.Z` tag, which fires `release.yml`. release-plz owns version + changelog + tag;
cargo-dist owns build + GitHub Release + Homebrew (`git_release_enable=false`,
`publish=false` — crates.io stays off). bot mints GitHub App installation
token (`toolu-release` App installed on this
repo; `TOOLU_RELEASE_CLIENT_ID` + `TOOLU_RELEASE_PRIVATE_KEY`;
each job's token narrowed with `permission-*` to what it does — see
`docs/release.md` § one-time setup) so pushed tag triggers downstream
workflows. 
`just release` recipe remains manual fallback. See `docs/release.md`.

## Claude Code Hooks

`.claude/settings.json` wires **only** toolu-conventions guardrails
entries. comemory's own dispatcher layer (`.claude/hooks/`, adapted from
`qwick-business-app`) is **retired** — one enforcement surface, not two:

- **PostToolUse** (`Edit|Write` matcher) — `bash scripts/guardrails/run.sh --hook`,
  file-addressable structure checks on just-edited path(s).
- **Stop** — `bash scripts/guardrails/run.sh --stop`, full repo-mode
  guardrails sweep behind two early-outs.

guardrails hook writes to stderr and exits **2** (Claude Code ignores 
`1` from hook; only `2` surfaces on `PostToolUse` or blocks on `Stop`).
That contract is why it was never routed through old dispatcher, which
swallowed stderr and always exited 0 — layer would have been inert while
looking correctly wired. It is now only entry, so question is moot.

### What retiring the dispatcher gave up, and what covers it now

dispatcher was *tool-call interceptor*; guardrails is *repo-state
checker*. They never overlapped, so this is real reduction rather than 
consolidation — recorded here so nobody assumes guardrails absorbed it:

| Retired check | What covers it now |
| --- | --- |
| `npm`/`bun`/`pip`/`uv` blocked (Rust project) | Nothing. Convention only. |
| `rm -rf`, `git reset --hard`, `git checkout .`, `chmod -R 777` blocked | harness's own confirmation on destructive commands. |
| `git push --force` blocked | Nothing — and note old rule also caught `--force-with-lease`, *safe* variant, so rebased branch had no sanctioned way to publish. |
| `--no-verify` / `--no-gpg-sign` blocked | `lefthook.yml` still runs on commit/push, and CI runs `scripts/check-all.sh` on ready PRs targeting `main` — local bypass cannot land. |
| Direct `rustfmt` / `cargo clippy` blocked outside `scripts/` | Nothing. `scripts/fmt-check.sh` and `scripts/lint-check.sh` remain canonical invocations. |
| `protected-files.sh` (build artifacts, `scripts/guardrails/**`) | Convention only — `scripts/guardrails/` is still copied verbatim from kit and must not be hand-edited; `guardrails.config.json` and `scripts/guardrails/patterns/rust/` remain sanctioned knobs. |
| `auto-format.sh` re-ran `rustfmt` on touched files | Deliberately gone. It invoked `rustfmt` without project's edition, so it reordered imports into form `scripts/fmt-check.sh` then rejected — it manufactured drift it existed to prevent. |
| `gate-status.sh` session gate tracking | Nothing. `bash scripts/check-all.sh` on demand. |
| `session-end.sh` ran `fmt-check` + `typos-check` at Stop | guardrails `--stop` sweep, plus lefthook's pre-commit `fmt`/`typos` jobs. |

load-bearing gates were never in this layer: `lefthook.yml` runs `fmt`,
`guardrails` and `typos` pre-commit and `check_all` pre-push, and CI runs
`scripts/check-all.sh` on ready PRs targeting `main`.

User-facing docs live under `docs/`, organized in Diátaxis tiers and indexed by
`docs/README.md`: `docs/getting-started.md` tutorial, task-oriented
`docs/guides/` how-tos (byo-vectors, auto-reindex, ranking-and-eval, http-api,
prune-and-gc), `docs/cli-reference.md` reference (every subcommand, flag, and
env var), and `docs/architecture.md` explanation. README is front door
that links into them.

## Deviations from toolu-conventions

Every deviation below is deliberate and documented here. `AGENTS.md` carries
repository guidance. `CLAUDE.md` is removed. None weakens kit's
intent; several make local rule strictly stronger than one it replaces.

- **D1 — `unsafe_code = "forbid"` is NOT set.** crate has 7 FFI-necessary
  `unsafe` blocks and 3 `unsafe extern "C" fn` items in
  `src/store/tokenizer/ffi.rs` and `src/store/connection.rs`: registering 
  custom FTS5 tokenizer through `libsqlite3-sys`'s C ABI and registering
  `sqlite-vec` as SQLite auto-extension have no safe wrapper in 
  ecosystem. One more block is `Command::pre_exec` in
  `src/domains/sync/daemon/spawn.rs`: closing descriptors detached
  coordinator inherited must run between `fork` and `exec`, which std only
  exposes as `unsafe` hook. `forbid` is hard rustc error with no local override, so it
  cannot be applied. key is omitted (rustc's default is `allow`) and
  replaced with stronger, machine-enforced rule instead:
  `scripts/guardrails/patterns/rust/no-unsafe-without-safety.yml` fails 
  gate on any `unsafe` block or `unsafe fn` lacking `// SAFETY:` line in 
  comment block directly above it — strictly stronger than bash check it
  replaced, which never inspected `unsafe fn` signature at all.
- **D2 — `fileSize.max` is 300, not kit's 500, with exactly one
  `fileSize.overrides` entry: 400 for `.github/workflows/release.yml`.**
  comemory's module decomposition is designed around 300 code lines. CORE
  permits stack to add rules but never relax one; 300 is stricter, so this
  is compliant. one override is scoped to `.github/workflows/release.yml`
  and raises ceiling for no other path — every other file in repo,
  `.github/workflows/release-finalize.yml` and `.github/workflows/test.yml`
  included, is still checked at 300. `release.yml` is generated by
  `cargo dist init` and carries hand-maintained blocks re-applied after
  every regeneration (Homebrew tap token step among them); repo cannot split
  it, so ceiling was unenforceable there and
  only ever fired in lefthook's file mode (repo-mode sweep scans
  `srcRoot: src` and never saw it), blocking any commit that touched 
  workflow. Every hand-written workflow stays under 300 default — 
  next largest is `.github/workflows/test.yml` at 198. `release.yml`
  measures 314 (2026-10-24), so override is what keeps it passing — and
  hand edits, not `cargo dist`, pushed it there from 295 (#181, #236,
  #300, #311, #401). override therefore also absorbs hand-written lines
  ceiling exists to catch: keep hand blocks in this file minimal. Gate's
  count is `GR_FS_AWK` in `scripts/guardrails/checks/file-size.sh` (it also
  skips `//`, `/* … */`, `*`-led and `---` lines, and counts `#[`/`#![`
  attributes as code), so re-measure with gate's own awk rather than 
  hand-rolled one:

      eval "$(sed -n "/^GR_FS_AWK=/,/^'\$/p" scripts/guardrails/checks/file-size.sh)"
      awk "$GR_FS_AWK" <file>

- **D3 — `barrelNames: ["mod.rs"]`** is inherited unchanged from pinned
  Rust conventions, not local deviation. It makes `<dir>.rs` beside
  `<dir>/` layout permanent and machine-checked.
- **D4 — `src.nested` replaces the starter map** with exact, project-specific
  folder policy below. It retains kit's test separation while replacing its
  generic `model`/`service`/`store` domain shape with comemory's staged
  capabilities and delivery adapters:

  - `domains`: `architecture`, `memories`, `code`, `documents`, `graph`, `retrieval`, `learning`, `sync`, `capture`, `maintenance`, `integrations`, `projects`
  - `domains/*`: `ast`, `cloud`, `consolidation`, `daemon`, `doctor`, `document`, `drain`, `evaluation`, `exchange`, `index_code`, `install`, `memory_store`, `replica`, `rebuild`, `repos`, `retention`, `setup`, `source`, `unified`, `upgrade`, `tests`, `proptest-regressions`
  - `store`: `tokenizer`, `migrate`, `tests`
  - `cli`: `output`, `tests`, `proptest-regressions`, `setup`
  - `serve`: `routes`, `jobs`, `tests`, `proptest-regressions`
  - `serve/routes`: `memories`, `maint`, `tests`, `proptest-regressions`
  - `*`: `tests`, `proptest-regressions`

  `serve` remains HTTP adapter, `mcp` is stdio MCP adapter, and
  `store` remains SQLite exception; no template name controls their
  project ownership.
- **D5 — `src.requireReadme` replaces the pinned single-`domains` rule during staging with exactly 47 folders.**
  `domains` entry returned with #167, first real capability domain to
  land, and every capability folder gains its own entry as it lands. It names
  folders, not files: each listed grown folder has `README.md` that indexes
  its contents, while single-file module is listed in its parent folder README
  and documented by its module doc. configured list, rather than prose
  count, is source of truth.
- **D6 — two project-local ast-grep pattern rules**
  (`no-unsafe-without-safety.yml` for D1, `no-allow-attribute.yml` for Binding
  Rule 5's `#[allow]` ban) are **additions** to `scripts/guardrails/patterns/rust/`;
  no kit file is modified, so future `cp -R` re-copy of kit merges
  rather than clobbers them.
- **D7 — eight `clippy::pedantic` lints start at `allow` with counted
  burn-down**, each line in `Cargo.toml [lints.clippy]` carrying measured
  count at migration time. `-D warnings` with hundreds of pre-existing
  warnings is not gate, it is red build; adopting `pedantic` while
  explicitly listing what is not yet met is honest and burnable. This is 
  declared crate policy in `Cargo.toml`, not `#[allow]` bypass in code —
  ban on in-code `#[allow]` is retained and machine-enforced by D6. See
  `docs/lint-debt.md` for full list and burn-down order.
- **D8 — extra quality gates are retained** beyond kit's four-step
  command (`fmt && clippy && guardrails && test`): `typos-check`,
  `cli-docs-check`, `coverage-check`, `eval-check`, and three tool-dependent
  gates `dup-check`, `machete-check` and `deny-check` — each pinned to one tool
  build by constant in its own script, and each wired into `test`
  workflow. None has kit equivalent; all guard
  comemory-specific failure modes. They layer around guardrails step, not
  in place of it. Mutation testing is no longer among them: its workflow ran
  nightly full sweep and PR-scoped job that had been disabled, and it was
  removed. `just mutation` still runs full sweep locally through
  `scripts/mutation-check.sh`, against `.cargo/mutants.toml` and 
  `tests/golden/mutant-baseline.md` baseline — tool stays, only 
  scheduled CI job is gone.
- **D9 — `tests/common/` stays at crate root** and is bridged into lib
  test crate via `src/test_common.rs`, so colocated unit tests and crate-root
  integration tests share one copy of every fixture (Binding Rule 1). Its
  `#![allow(dead_code)]` is migration's one sanctioned exception,
  exempted by filename in `no-allow-attribute.yml`.
- **D10 — `extern crate self as comemory;` is added to `src/lib.rs`.** This
  lets every colocated test file keep `use comemory::...` imports identical
  to crate-root suite's, so test reads same wherever it lives.
- **D11 — 106 of 300 test files remain crate-root integration tests**
  (61 CLI surfaces, 80 driving real binary via `assert_cmd`, 3 owning 
  `insta` snapshot) — kit's own "one file per public surface" category,
  not exception, at ratio high enough to declare. ratio is hard
  floor, not inertia: see "Why crate-root `tests/` cannot be deleted" under
  Testing for `CARGO_BIN_EXE_comemory` constraint that pins it.
- **D12 — RESOLVED, no longer deviation.** `.claude/settings.json` was
  briefly *merge* of comemory's `pre-tools`/`post-tools`/`session-end`
  dispatchers and kit's two guardrails entries. dispatcher layer is
  now retired and file wires kit's entries alone, so this file matches
  kit and there is nothing left to declare. See "Claude Code Hooks" above
  for what dispatcher used to catch and what covers each item now — 
  reduction is real, and load-bearing gates (`lefthook.yml`, CI's
  `scripts/check-all.sh`) were never in that layer.
- **D13 — `benches/` is untouched.** kit's STRUCTURE.md is silent on
  benches; Criterion harnesses have their own cargo semantics and are neither
  `src/` nor `tests/`. They carry only canonical test/bench lint header.
- **D14 — `[workspace] members = ["."]` stanza stays**, required by
  `cargo-dist`. guardrails module keys workspace mode on presence of
  `guardrails.workspace.json`, not on Cargo metadata, so this repo takes 
  single-repo path regardless.

## Agent integration

`comemory install claude` and `comemory install codex` install bundled skills
and hooks through host plugin manager. Use `--dry-run` to preview. Prefer
MCP `find` with `k: 3`, then `show` selected memories; `context` returns full
bodies without token cap. Stop reminders summarize shared repo activity and
never block session. Transcript capture currently supports Claude Code only.
integration is owned here and requires no toolu plugin. See
[installation and migration](docs/guides/agent-integration.md).
