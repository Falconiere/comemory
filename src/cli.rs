//! Clap-driven CLI surface for the memory layer. The dispatcher in `run`
//! delegates to one-file-per-subcommand modules so each command owns its own
//! argument shape and output rendering.

use clap::{Parser, Subcommand};

use crate::config::Config;
use crate::config::paths::Paths;
use crate::prelude::*;

/// `comemory architecture`: the component-level architecture model.
pub mod architecture;
/// `comemory ast`: user-facing ast-grep pattern search.
pub mod ast;
/// `comemory auth`: cloud workspace-key login / status / logout.
pub mod auth;
/// TTY and JSON rendering for authentication commands.
pub mod auth_render;
/// `comemory backup`: create, restore and merge-erasures (CLI-only).
pub mod backup;
/// `comemory bandit`: Thompson sampling over the tune knobs.
pub mod bandit;
/// `comemory benchmark` — the domain-aware offline retrieval benchmark.
pub mod benchmark;
/// `comemory capture`: session receipt + consent read (CLI-only).
pub mod capture;
mod completion_install;
/// Completion-script generation shared by the CLI and `GET /api/v1/completions`.
pub mod completion_script;
/// `comemory completions`: shell completion scripts.
pub mod completions;
/// `comemory consolidate`: advisory near-duplicate cluster report.
pub mod consolidate;
/// `comemory context`: headline memory + code bundle for a query.
pub mod context;
/// The required resident daemon's per-command classification and preflight.
mod daemon_preflight;
/// `comemory delete`: soft-delete one memory.
pub mod delete;
/// `comemory distill`: extract explicit saves and propose platform candidates.
pub mod distill;
/// `comemory doctor`: runtime health check.
pub mod doctor;
/// `comemory edges`: lexical search over the relation graph.
pub mod edges;
/// `comemory erase`: permanently erase one memory or document.
pub mod erase;
/// `comemory eval`: score retrieval against a golden set.
pub mod eval;
/// `comemory export-dataset`: the reviewed relevance dataset export.
pub mod export_dataset;
/// `comemory feedback`: record which hits were used.
pub mod feedback;
/// `comemory find` — one ranked list across memory, code, and documents.
pub mod find;
/// `comemory gc`: retention sweep over the learning tables.
pub mod gc;
/// `comemory graph`: walk the relation graph by id.
pub mod graph;
/// `comemory hooks` — read and toggle the git reindex hooks.
pub mod hooks;
/// `comemory index`: register document sources and reconcile them.
pub mod index;
/// `comemory index-code`: extract and index code symbols.
pub mod index_code;
/// `comemory ingest-code`: bulk code ingestion.
pub mod ingest_code;
/// `comemory install`: standalone agent skills and hooks.
pub mod install;
/// `comemory install-hooks`: git-hook installation.
pub mod install_hooks;
/// `comemory judge`: reviewed relevance verdicts against a captured
/// candidate observation.
pub mod judge;
/// Detached auto-reindex spawn behind `indexing.auto_reindex = lazy`.
pub mod lazy_reindex;
/// `comemory list`: page live memories.
pub mod list;
/// `comemory mcp`: the stdio Model Context Protocol tool surface.
pub mod mcp;
/// `comemory mine`: distill query reformulations into expansions.
pub mod mine;
/// Run blocking platform calls outside the async runtime.
pub mod off_runtime;
/// TTY and JSON writers shared by the subcommands (`cli::output::*`).
pub mod output;
/// Shared `--k` / `--offset` window resolution.
pub mod pagination;
/// `comemory project` — engine-owned project management (create, show, list,
/// archive, restore, pause, resume, activity, changes).
pub mod project;
/// `comemory project activity` — the flags and TTY view of one project's
/// activity page.
pub mod project_activity;
/// `comemory project evidence add|list` — the flags and TTY views of typed
/// evidence.
pub mod project_evidence;
/// `comemory project archive|restore|pause|resume` — the flags the four
/// lifecycle verbs share.
pub mod project_lifecycle;
/// `comemory prune`: orphan / low-value / stale-code candidates.
pub mod prune;
/// `comemory rebuild`: reconstruct the store from markdown.
pub mod rebuild;
/// `comemory recall-status`: tracked queries, verdicts, saves and pending
/// recalls for a repo + lower time bound.
pub mod recall_status;
/// `comemory repos` — the indexed code-repository inventory.
pub mod repos;
/// `comemory save`: write a memory (markdown + store mirror).
pub mod save;
/// `comemory search`: hybrid memory retrieval.
pub mod search;
/// `comemory search-code`: ranked code search.
pub mod search_code;
/// `--only`/`--path` domain-scope resolution + the interim
/// `--only document` search path, shared by `search`.
pub(crate) mod search_only;
/// `comemory serve`: loopback web viewer.
pub mod serve;
/// `comemory setup`: guided first-run onboarding.
pub mod setup;
/// `comemory show` — one memory in full.
pub mod show;
/// `comemory sources`: list registered document sources.
pub mod sources;
/// `comemory stats` — corpus counters and database size.
pub mod stats;
/// `comemory sync`: push/pull against the platform (CLI-only).
pub mod sync;
/// `comemory sync --action auto`: the hook-fired pass and its `--json`.
pub mod sync_auto;
/// `comemory sync daemon`: the required resident coordinator's lifecycle.
pub mod sync_daemon;
/// Rendering of the `exchange` state for `comemory sync`.
pub mod sync_exchange_render;
/// Rendering for `comemory sync` (TTY + `--json` shapes).
pub mod sync_render;
/// `comemory tune`: grid/sampled search over the ranking knobs.
pub mod tune;
/// `comemory unindex`: unregister a document source and remove its
/// derived rows.
pub mod unindex;
/// `comemory upgrade`: move this binary to a newer release.
pub mod upgrade;
/// `comemory watch`: follow workspace changes through the coordinator.
pub mod watch;

/// Top-level CLI. `comemory <subcommand> [--json] [--data-dir DIR]`. The `--json`
/// and `--data-dir` flags are global so callers can place them either before
/// or after the subcommand.
#[derive(Parser, Debug)]
#[command(
    name = "comemory",
    version,
    about = "Agentic dev memory + code-aware semantic search",
    arg_required_else_help = true
)]
pub struct Cli {
    /// Emit machine-readable JSON instead of a human TTY view.
    #[arg(long, global = true)]
    pub json: bool,

    /// Override the data root (defaults to `$HOME/.comemory`). Honors the
    /// `COMEMORY_DATA_DIR` environment variable.
    #[arg(long, global = true, env = "COMEMORY_DATA_DIR")]
    pub data_dir: Option<std::path::PathBuf>,

    /// The subcommand to run.
    #[command(subcommand)]
    pub cmd: Cmd,
}

/// Memory-layer + code-layer subcommands. Clap derives the kebab-case name
/// from each variant, so `IndexCode` becomes `index-code`, `MemoryFor` becomes
/// `memory-for`, etc.
#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Scaffold, store, draw and drift-check the architecture model of an
    /// indexed repository (CLI-only).
    Architecture(architecture::Args),
    /// Save a memory (body via arg, `-`, or stdin).
    Save(save::Args),
    /// Engine-owned project management: create, show and list project
    /// charters, archive, restore, pause and resume them, page a project's
    /// activity log and read the body-free change feed, offline.
    Project(project::Args),
    /// Search the memory index by natural-language query.
    Search(search::Args),
    /// Search the code index by natural-language or identifier query.
    SearchCode(search_code::Args),
    /// List memories with optional repo/kind filters.
    List(list::Args),
    /// Soft-delete a memory by id (moves to `.trash/`).
    Delete(delete::Args),
    /// Extract explicit `comemory save` claims from a transcript and propose
    /// them as platform candidate memories (CLI-only).
    Distill(distill::Args),
    /// Record per-memory feedback (used vs irrelevant).
    Feedback(feedback::Args),
    /// Score retrieval quality against a golden set (recall@k, MRR).
    Eval(eval::Args),
    /// Score a reviewed benchmark set over memory, code and document
    /// retrieval, and emit a replayable candidate-observation artifact.
    Benchmark(benchmark::Args),
    /// Record reviewed relevance verdicts against a captured candidate
    /// observation, or report that observation (CLI-only).
    Judge(judge::Args),
    /// Export the reviewed relevance dataset and its manifest as versioned
    /// JSONL, with grouped splits and a withheld holdout (CLI-only).
    ExportDataset(export_dataset::Args),
    /// Mine reformulation pairs from the query log into term-expansion
    /// mappings (report only; `--apply` rebuilds `query_expansions`).
    Mine(mine::Args),
    /// Grid-search blend weights against the golden set (report only;
    /// `--apply` writes the winner into config.toml).
    Tune(tune::Args),
    /// Thompson-sample blend knobs against the golden set (report only;
    /// `--apply` writes when the sample beats baseline).
    Bandit(bandit::Args),
    /// Report on the data directory and SQLite mirror health.
    Doctor(doctor::Args),
    /// Walk a repo, extract symbols, and upsert into the code index.
    IndexCode(index_code::Args),
    /// Read pre-embedded JSONL rows from stdin and ingest them into the code
    /// index (`code_symbols` + `code_fts` + `code_vec`).
    IngestCode(ingest_code::Args),
    /// Register one or more paths as document sources and reconcile them.
    Index(index::Args),
    /// List registered document sources with per-status counts.
    Sources(sources::Args),
    /// Report corpus counters and the size of `comemory.db`.
    Stats(stats::Args),
    /// List indexed code repositories with their index freshness.
    Repos(repos::Args),
    /// Show one memory in full: body, frontmatter, activation, references.
    Show(show::Args),
    /// Search memories, code, and documents as one ranked list.
    Find(find::Args),
    /// Report and toggle the git hooks that trigger background reindexing.
    Hooks(hooks::Args),
    /// Unregister a document source and remove its derived rows.
    Unindex(unindex::Args),
    /// Run an ast-grep pattern against a single source file.
    Ast(ast::Args),
    /// Export the file-level code-connection graph (imports + co-change)
    /// as JSON, Graphviz DOT, or an interactive HTML page.
    Graph(graph::Args),
    /// Search the relation graph lexically (supersedes, imports, references).
    Edges(edges::Args),
    /// Serve the loopback HTTP API (`/api/v1`) for consoles, agents, and scripts.
    Serve(serve::Args),
    /// Serve the MCP tool interface over stdio for agent hosts.
    Mcp(mcp::Args),
    /// Detect what this machine and repo still need, then set it up.
    Setup(setup::Args),
    /// Headline lookup: code symbol + memories matching a key.
    Context(context::Args),
    /// Emit a shell completion script, or install completions for supported shells.
    Completions(completions::Args),
    /// Detect (and optionally soft-delete) stale memories.
    Prune(prune::Args),
    /// Report near-duplicate memory clusters and the member worth keeping.
    Consolidate(consolidate::Args),
    /// Drop `comemory.db` and repopulate it from the markdown source of truth.
    Rebuild(rebuild::Args),
    /// Back up the data directory, restore a backup under a new stream epoch,
    /// or merge an erasure manifest a restore could not find (CLI-only).
    Backup(backup::Args),
    /// Report tracked recalls awaiting a verdict, verdicts and saves since a
    /// bound.
    RecallStatus(recall_status::Args),
    /// Purge old `memories/.trash/` entries and learning telemetry past
    /// retention.
    #[command(after_help = gc::EXAMPLES)]
    Gc,
    /// Permanently erase one memory or document from every table, journal
    /// copy and the markdown tree, keeping only the digests that refuse its
    /// bytes afterwards.
    Erase(erase::Args),
    /// Install git hooks that index and sync the repo on `post-commit`,
    /// `post-merge`, `post-checkout` and `post-rewrite`.
    InstallHooks(install_hooks::Args),
    /// Install bundled skills and hooks for Claude Code or Codex.
    Install(install::Args),
    /// Move this binary to the newest release (or a pinned one).
    Upgrade(upgrade::Args),
    /// Cloud workspace-key login / status / logout (device authorization).
    Auth(auth::Args),
    /// Push/pull memories against the platform.
    Sync(sync::Args),
    /// Follow the organization's changes over the workspace channel.
    Watch(watch::Args),
    /// Capture a coding-session receipt (redacted) to the platform.
    Capture(capture::Args),
}

/// Recover interrupted writes before dispatch, outside the store layer.
/// Skip absent/newer databases and daemon lifecycle commands. Serve and MCP
/// reconcile during their own startup, respecting their read-only option.
fn reconcile_pending(data_dir: Option<&std::path::Path>, cmd: &Cmd) -> Result<()> {
    if matches!(
        cmd,
        Cmd::Sync(sync::Args {
            cmd: Some(sync::SyncCmd::Daemon(_)),
            ..
        })
    ) {
        return Ok(());
    }
    let paths = Paths::new(crate::config::paths::resolve_data_dir(
        data_dir.map(std::path::Path::to_path_buf),
    ));
    if !paths.db_path().exists() {
        return Ok(());
    }
    let mut conn = match crate::store::connection::open(paths.db_path()) {
        Ok(conn) => conn,
        // A database written by a newer build is one this binary must not
        // touch. Returning here leaves the forward-compat contract to the
        // subcommand, which is what owns it: `doctor` falls back to a
        // read-only report, every other command exits 70 naming the unknown
        // migration key. Any other open failure propagates.
        Err(Error::SchemaTooNew(_)) => return Ok(()),
        Err(e) => return Err(e),
    };
    let report = crate::domains::memories::recover::reconcile(&paths, &mut conn)?;
    if !report.is_empty() {
        tracing::info!(
            finished = report.finished,
            dropped = report.dropped,
            "recovered memory writes an interrupted run left outstanding"
        );
    }
    Ok(())
}

/// Dispatch the parsed `Cli` to its subcommand. The dispatcher is the single
/// place that knows about every variant, keeping individual subcommand modules
/// free of cross-references.
pub async fn run(cli: Cli) -> Result<()> {
    match prepare_command(cli.cmd, cli.data_dir.as_deref())? {
        Cmd::Architecture(a) => architecture::run(a, cli.json, cli.data_dir).await,
        Cmd::Save(a) => save::run(a, cli.json, cli.data_dir).await,
        Cmd::Project(a) => project::run(a, cli.json, cli.data_dir).await,
        Cmd::Search(a) => search::run(a, cli.json, cli.data_dir).await,
        Cmd::SearchCode(a) => search_code::run(a, cli.json, cli.data_dir).await,
        Cmd::List(a) => list::run(a, cli.json, cli.data_dir).await,
        Cmd::Delete(a) => delete::run(a, cli.json, cli.data_dir).await,
        Cmd::Distill(a) => distill::run(a, cli.json, cli.data_dir).await,
        Cmd::Feedback(a) => feedback::run(a, cli.json, cli.data_dir).await,
        Cmd::Eval(a) => eval::run(a, cli.json, cli.data_dir).await,
        Cmd::Benchmark(a) => benchmark::run(a, cli.json, cli.data_dir).await,
        Cmd::Judge(a) => judge::run(a, cli.json, cli.data_dir).await,
        Cmd::ExportDataset(a) => export_dataset::run(a, cli.json, cli.data_dir).await,
        Cmd::Mine(a) => mine::run(a, cli.json, cli.data_dir).await,
        Cmd::Tune(a) => tune::run(a, cli.json, cli.data_dir).await,
        Cmd::Bandit(a) => bandit::run(a, cli.json, cli.data_dir).await,
        Cmd::Doctor(a) => doctor::run(a, cli.json, cli.data_dir).await,
        Cmd::IndexCode(a) => index_code::run(a, cli.json, cli.data_dir).await,
        Cmd::IngestCode(a) => ingest_code::run(a, cli.json, cli.data_dir).await,
        Cmd::Index(a) => index::run(a, cli.json, cli.data_dir).await,
        Cmd::Sources(a) => sources::run(a, cli.json, cli.data_dir).await,
        Cmd::Stats(a) => stats::run(a, cli.json, cli.data_dir).await,
        Cmd::Repos(a) => repos::run(a, cli.json, cli.data_dir).await,
        Cmd::Show(a) => show::run(a, cli.json, cli.data_dir).await,
        Cmd::Find(a) => find::run(a, cli.json, cli.data_dir).await,
        Cmd::Hooks(a) => hooks::run(a, cli.json, cli.data_dir).await,
        Cmd::Unindex(a) => unindex::run(a, cli.json, cli.data_dir).await,
        Cmd::Ast(a) => ast::run(a, cli.json, cli.data_dir).await,
        Cmd::Graph(a) => graph::run(a, cli.json, cli.data_dir).await,
        Cmd::Edges(a) => edges::run(a, cli.json, cli.data_dir).await,
        Cmd::Serve(a) => serve::run(a, cli.json, cli.data_dir).await,
        Cmd::Mcp(a) => mcp::run(a, cli.json, cli.data_dir).await,
        Cmd::Setup(a) => setup::run(a, cli.json, cli.data_dir).await,
        Cmd::Context(a) => context::run(a, cli.json, cli.data_dir).await,
        Cmd::Completions(a) => completions::run(a, cli.json, cli.data_dir).await,
        Cmd::Prune(a) => prune::run(a, cli.json, cli.data_dir).await,
        Cmd::Consolidate(a) => consolidate::run(a, cli.json, cli.data_dir),
        Cmd::Rebuild(a) => rebuild::run(a, cli.json, cli.data_dir).await,
        Cmd::Backup(a) => backup::run(a, cli.json, cli.data_dir).await,
        Cmd::RecallStatus(a) => recall_status::run(a, cli.json, cli.data_dir).await,
        Cmd::Gc => gc::run(cli.json, cli.data_dir).await,
        Cmd::Erase(a) => erase::run(a, cli.json, cli.data_dir).await,
        Cmd::InstallHooks(a) => install_hooks::run(a, cli.json, cli.data_dir).await,
        Cmd::Install(a) => install::run(a, cli.json, cli.data_dir),
        Cmd::Upgrade(a) => upgrade::run(a, cli.json, cli.data_dir).await,
        Cmd::Auth(a) => auth::run(a, cli.json, cli.data_dir).await,
        Cmd::Sync(a) => sync::run(a, cli.json, cli.data_dir).await,
        Cmd::Watch(a) => watch::run(a, cli.json, cli.data_dir).await,
        Cmd::Capture(a) => capture::run(a, cli.json, cli.data_dir).await,
    }
}

/// Prepare the coordinator and recover pending writes before dispatch.
fn prepare_command(cmd: Cmd, data_dir: Option<&std::path::Path>) -> Result<Cmd> {
    daemon_preflight::run(data_dir, &cmd)?;
    // Serve and MCP reconcile during their own startup, which knows whether
    // the session is read-only. Daemon lifecycle commands never reconcile.
    if !matches!(cmd, Cmd::Serve(_) | Cmd::Mcp(_)) {
        reconcile_pending(data_dir, &cmd)?;
    }
    Ok(cmd)
}

/// Load the layered config: defaults → optional `config.toml` → env. Every
/// CLI entry point goes through this helper so the file layer cannot silently
/// drop out for one subcommand (which would cause `comemory doctor` and
/// `comemory search` to disagree on the effective config).
pub(crate) fn load_config(paths: &Paths) -> Result<Config> {
    Config::defaults()
        .with_file(paths.config_file().as_path())?
        .with_env()
}
