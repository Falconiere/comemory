# cli/

**What belongs here:** clap subcommand entry points — one file per
`comemory <subcommand>`, each owning its own `Args` shape, thin orchestration
(`run`), and output rendering hookup — plus the top-level dispatcher
(`Cli`/`Cmd` in `src/cli.rs`), the small cross-cutting flag layers shared by
several subcommands (`pagination`, `search_only`), and the TTY/JSON writers in
`output/`, which #178 moved in from the top level. The transport-neutral
helpers that used to live here (`when`, `ref_args`, `embedding_input`) moved to
`utilities::` with #166.

**What does NOT belong here:** business logic. A `cli/*.rs` file parses flags,
loads `Config`, calls into `domains::` or `store::` to do the real work, and
hands the result to `cli::output::` to render. (It is `domains::` throughout:
`cli/prune.rs` calls `domains::maintenance::prune`, never a top-level
`prune::` — that crate-root name is only the compatibility alias `lib.rs`
keeps over `domains::maintenance::retention` for library consumers.)
Keeping the logic out of `cli/` is what lets `domains::learning::evaluation::runner` and tests exercise
the same pipelines without going through argument parsing. CLI integration
tests stay at crate-root (`tests/cli__*.rs` per command, `tests/cli_scenario_*.rs`
for multi-command journeys) — never under `src/cli/`.

## Contents

One line per file, named after its primary item:

| File | Primary item | Purpose |
| --- | --- | --- |
| `architecture.rs` | `Args` | `comemory architecture` — scaffold / save / show / check / learn the component-level architecture model (CLI-only); every core lives in `domains::architecture` |
| `ast.rs` | `Args` | `comemory ast` — run an ast-grep pattern against a source file |
| `auth.rs` | `Args` | `comemory auth` — nested `login` / `status` / `logout` for the org-scoped key; the sequences are `domains::sync::login`, and `login` still runs the first sync (CLI-only) |
| `auth_render.rs` | `LoginJson` | JSON/TTY helpers for `comemory auth` login/status/logout |
| `bandit.rs` | `Args` | `comemory bandit` — Thompson-sample the `[tune]` grid, confirm with offline eval |
| `completion_install.rs` | `Report` | Per-user Bash, Zsh, Fish and PowerShell completion installation plus idempotent profile registration |
| `completion_script.rs` | `Request` | Completion-script generation shared by `comemory completions` and `GET /api/v1/completions` |
| `completions.rs` | `Args` | `comemory completions <shell>` emits a script; `--install` installs and registers the four supported interactive shells |
| `capture.rs` | `Args` | `comemory capture` — post a session transcript's receipts and candidate batches over the device-key credential; core in `domains::capture` |
| `consolidate.rs` | `Args` | `comemory consolidate` — advisory near-duplicate cluster report |
| `context.rs` | `Args` | `comemory context` — headline memory + code bundle lookup |
| `delete.rs` | `Args` | `comemory delete` — soft-delete a memory by id |
| `distill.rs` | `Args` | `comemory distill` — extract explicit saves and propose platform candidates (CLI-only) |
| `doctor.rs` | `Args` | `comemory doctor` — runtime health check against the SQLite storage stack |
| `edges.rs` | `Args` | `comemory edges` — lexical search over the relation graph |
| `benchmark.rs` | `Args` | `comemory benchmark` — score a reviewed benchmark set over memory, code and document retrieval, print the arm summary, and write the replayable artifact |
| `eval.rs` | `GoldenSetArgs` | `comemory eval` — score retrieval quality (recall@k, MRR) against a golden set |
| `feedback.rs` | `Args` | `comemory feedback` — record used/irrelevant feedback into the stats DB |
| `gc.rs` | `run` | `comemory gc` — purge `.trash/` and evict expired learning telemetry |
| `find.rs` | `Args` | `comemory find` — one ranked list over memories, code, and documents |
| `graph.rs` | `Format` | `comemory graph` — the clap surface and the `--format` renderers; the graph itself is built by `domains::graph::query` |
| `hooks.rs` | `Args` | `comemory hooks` — report and toggle the git reindex hooks individually |
| `index.rs` | `Args` | `comemory index <PATH>...` — register document sources and reconcile them |
| `index_code.rs` | `Args` | `comemory index-code` — incremental symbol extraction over a git repo |
| `ingest_code.rs` | `Args` | `comemory ingest-code` — bulk pre-embedded code-symbol ingestion from stdin |
| `install.rs` | `Args` | `comemory install` — install the embedded agent skills and hooks through the native Claude Code or Codex plugin manager; core in `domains::integrations::install` |
| `install_hooks.rs` | `Args` | `comemory install-hooks` — install the four git hooks that run `sync --action auto`, then run the shipped hook body once in the repo (`kicked`) so it is indexed and synced without a commit — the body comes from the binary, never from the repo's hooks directory |
| `export_dataset.rs` | `Args` | `comemory export-dataset` — write the reviewed relevance dataset and its manifest as versioned JSONL, with grouped splits, a withheld holdout and the TTY summary of everything the export refused |
| `judge.rs` | `Args` | `comemory judge` — record reviewed relevance verdicts against a captured candidate observation, or report that observation |
| `lazy_reindex.rs` | `RepoContext` | Detached, non-blocking auto-reindex trigger behind `indexing.auto_reindex = lazy` |
| `list.rs` | `Args` | `comemory list` — page live memories with `--repo` / `--kind` filters |
| `off_runtime.rs` | `off_runtime` | Run blocking platform I/O on a scoped thread, away from the tokio runtime |
| `mcp.rs` | `Args` | `comemory mcp` — serve the MCP tool interface over stdio for agent hosts; the session itself is `mcp::serve` |
| `mine.rs` | `Args` | `comemory mine` — distill query reformulations from `retrieval_log` into expansions |
| `pagination.rs` | `PaginationArgs` | Shared `--k` / `--offset` window flags, flattened into paginated commands |
| `prune.rs` | `Args` | `comemory prune` — surface deletion candidates against the SQLite mirror |
| `rebuild.rs` | `Args` | `comemory rebuild` — atomically rebuild the SQLite mirror from markdown |
| `recall_status.rs` | `Args` | `comemory recall-status` — tracked queries, verdicts, saves and pending recalls for a repo + lower time bound; core in `domains::learning::recall_status` |
| `save.rs` | `Args` | `comemory save` — atomic markdown write + SQLite-mirror upsert; waits on the after-save push |
| `search.rs` | `Args` | `comemory search` — natural-language search over the memory store |
| `search_code.rs` | `Args` | `comemory search-code` — ranked search over indexed `code_symbols` |
| `output/` | — | TTY and JSON writers shared by the subcommands; see `output/README.md` |
| `search_only.rs` | `OnlyDomain` | The `--only` clap `ValueEnum` and the interim `--only document` path; the resolution policy itself is `domains::retrieval::scope::resolve_domains` |
| `serve.rs` | `Args` | `comemory serve` — launch the loopback `/api/v1` HTTP server (API-only; the embedded web viewer was removed in 0.18.0). Also owns the startup banner: `serve::serve` hands back a `serve::Ready` and this module writes the `--json` payload or the TTY header, so the server never touches stdout |
| `repos.rs` | `Args` | `comemory repos` — indexed code repositories and their index freshness |
| `setup.rs` | `Args` | `comemory setup` — detect, plan, and apply first-run onboarding; core in `domains::integrations::setup`. Owns the `Intent`/`Prompting` → `Mode` decision and the exit-code mapping; `setup/` holds the wizard and the summary renderer |
| `show.rs` | `Args` | `comemory show` — one memory in full: body, frontmatter, activation, refs |
| `sources.rs` | `Args` | `comemory sources` — list registered document sources with status counts |
| `stats.rs` | `Args` | `comemory stats` — corpus counters and `comemory.db` size |
| `sync.rs` | `Args` | `comemory sync` — `--action` `push` / `pull` / `status` / `verify` for cloud sync (the run sequences are `domains::sync::manual`, and `run`/`push` still push the code index after the memories), plus the nested `daemon` subcommand (`sync_daemon.rs`) |
| `sync_auto.rs` | `run` | `comemory sync --action auto` — dispatched before any login is required. With the required coordinator (#257) sends one wake over its control socket and returns; only under the `COMEMORY_SYNC_DAEMON=0` harness switch does it fall back to `domains::sync::auto::run_auto` off the runtime. Prints its `--json` report either way (nothing without `--json`) |
| `sync_daemon.rs` | `DaemonCmd` | `comemory sync daemon {ensure,status,restart,repair,stop,uninstall,run}` — the required resident coordinator's lifecycle (#257); `install`/`start` are hidden deprecated aliases of `repair`/`ensure`. `run` dispatches to `domains::sync::daemon::run_foreground`; every other verb calls `domains::sync::daemon::{ensure,status_view}` off the runtime |
| `daemon_preflight.rs` | `run` | The first thing `cli::run` does for every command: an exhaustive `Cmd` match classifies each `Exempt` / `BestEffort` / `Required` and verifies/repairs the coordinator accordingly, failing a `Required` command with a named local-service error (exit 69) if none answers. A no-op under `COMEMORY_SYNC_DAEMON=0` |
| `sync_exchange_render.rs` | `exchange_status_lines` | The TTY lines of `status`'s `exchange` block (protocol, network, cursor against the upstream head, outbox and pull holds by reason, a stall) and the one-line summary of a run's `exchange` leg |
| `sync_render.rs` | `emit_run` | The TTY and `--json` shapes of `comemory sync`: the run counters (memories and code), `status`'s cursors plus one `code` row per indexed repo (`moved_since_push`, and `withheld` when the row's root is a linked worktree, gone, or not a checkout), the verify report, the daemon status, and the login report's code line |
| `watch.rs` | `Args` | `comemory watch` — the one long-lived command. Attaches to the required resident coordinator (#257): subscribes over its control socket, `--once` adds one `catch_up`, following reports `pulled` per `pass_finished`. Only under `COMEMORY_SYNC_DAEMON=0` does it fall back to `domains::sync::watch::follow`, holding the platform's workspace channel itself; this module supplies the `OffRuntime` that fallback isolates blocking platform calls through |
| `tune.rs` | `Args` | `comemory tune` — deterministic/sampled search over the blend knobs |
| `unindex.rs` | `Args` | `comemory unindex <SOURCE_ID\|PATH>` — unregister a document source |
| `upgrade.rs` | `Args` | `comemory upgrade` — move this binary to the newest release (`--check`, `--version`, `--force`); core in `crate::domains::maintenance::upgrade`, CLI-only |

When you add a file here, add its row above so the index stays current. No
`mod.rs` barrel — submodules are declared from `src/cli.rs` (`pub mod <name>;`)
and the dispatcher (`Cmd`) imports concrete paths.
