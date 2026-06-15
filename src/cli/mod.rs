//! Clap-driven CLI surface for the memory layer. The dispatcher in `run`
//! delegates to one-file-per-subcommand modules so each command owns its own
//! argument shape and output rendering.

use clap::{Parser, Subcommand};

use crate::config::Config;
use crate::config::paths::Paths;
use crate::prelude::*;

pub mod ast;
pub mod completions;
pub mod context;
pub mod delete;
pub mod doctor;
pub(crate) mod embedding_input;
pub mod eval;
pub mod feedback;
pub mod gc;
pub mod graph;
pub mod index_code;
pub mod ingest_code;
pub mod install_hooks;
pub mod lazy_reindex;
pub mod list;
pub mod mine;
pub mod pagination;
pub mod prune;
pub mod rebuild;
pub mod save;
pub mod search;
pub mod search_code;
pub mod serve;
pub mod tui;
pub mod tune;

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

    #[command(subcommand)]
    pub cmd: Cmd,
}

/// Memory-layer + code-layer subcommands. Clap derives the kebab-case name
/// from each variant, so `IndexCode` becomes `index-code`, `MemoryFor` becomes
/// `memory-for`, etc.
#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Save a memory (body via arg, `-`, or stdin).
    Save(save::Args),
    /// Search the memory index by natural-language query.
    Search(search::Args),
    /// Search the code index by natural-language or identifier query.
    SearchCode(search_code::Args),
    /// List memories with optional repo/kind filters.
    List(list::Args),
    /// Soft-delete a memory by id (moves to `.trash/`).
    Delete(delete::Args),
    /// Record per-memory feedback (used vs irrelevant).
    Feedback(feedback::Args),
    /// Score retrieval quality against a golden set (recall@k, MRR).
    Eval(eval::Args),
    /// Mine reformulation pairs from the query log into term-expansion
    /// mappings (report only; `--apply` rebuilds `query_expansions`).
    Mine(mine::Args),
    /// Grid-search blend weights against the golden set (report only;
    /// `--apply` writes the winner into config.toml).
    Tune(tune::Args),
    /// Report on the data directory and SQLite mirror health.
    Doctor(doctor::Args),
    /// Walk a repo, extract symbols, and upsert into the code index.
    IndexCode(index_code::Args),
    /// Read pre-embedded JSONL rows from stdin and ingest them into the code
    /// index (`code_symbols` + `code_fts` + `code_vec`).
    IngestCode(ingest_code::Args),
    /// Run an ast-grep pattern against a single source file.
    Ast(ast::Args),
    /// Export the file-level code-connection graph (imports + co-change)
    /// as JSON, Graphviz DOT, or an interactive HTML page.
    Graph(graph::Args),
    /// Launch the local web viewer + in-browser code editor (loopback HTTP).
    Serve(serve::Args),
    /// Launch the read-only interactive terminal explorer.
    Tui(tui::Args),
    /// Headline lookup: code symbol + memories matching a key.
    Context(context::Args),
    /// Emit a shell completion script for `bash`, `zsh`, `fish`, `powershell`, or `elvish`.
    Completions(completions::Args),
    /// Detect (and optionally soft-delete) stale memories.
    Prune(prune::Args),
    /// Drop `comemory.db` and repopulate it from the markdown source of truth.
    Rebuild(rebuild::Args),
    /// Purge old `memories/.trash/` entries and learning telemetry past
    /// retention.
    #[command(after_help = gc::EXAMPLES)]
    Gc,
    /// Install git hooks that trigger `comemory index-code` on
    /// `post-commit`, `post-merge`, and `post-checkout`.
    InstallHooks(install_hooks::Args),
}

/// Dispatch the parsed `Cli` to its subcommand. The dispatcher is the single
/// place that knows about every variant, keeping individual subcommand modules
/// free of cross-references.
pub async fn run(cli: Cli) -> Result<()> {
    match cli.cmd {
        Cmd::Save(a) => save::run(a, cli.json, cli.data_dir).await,
        Cmd::Search(a) => search::run(a, cli.json, cli.data_dir).await,
        Cmd::SearchCode(a) => search_code::run(a, cli.json, cli.data_dir).await,
        Cmd::List(a) => list::run(a, cli.json, cli.data_dir).await,
        Cmd::Delete(a) => delete::run(a, cli.json, cli.data_dir).await,
        Cmd::Feedback(a) => feedback::run(a, cli.json, cli.data_dir).await,
        Cmd::Eval(a) => eval::run(a, cli.json, cli.data_dir).await,
        Cmd::Mine(a) => mine::run(a, cli.json, cli.data_dir).await,
        Cmd::Tune(a) => tune::run(a, cli.json, cli.data_dir).await,
        Cmd::Doctor(a) => doctor::run(a, cli.json, cli.data_dir).await,
        Cmd::IndexCode(a) => index_code::run(a, cli.json, cli.data_dir).await,
        Cmd::IngestCode(a) => ingest_code::run(a, cli.json, cli.data_dir).await,
        Cmd::Ast(a) => ast::run(a, cli.json, cli.data_dir).await,
        Cmd::Graph(a) => graph::run(a, cli.json, cli.data_dir).await,
        Cmd::Serve(a) => serve::run(a, cli.json, cli.data_dir).await,
        Cmd::Tui(a) => tui::run(a, cli.json, cli.data_dir).await,
        Cmd::Context(a) => context::run(a, cli.json, cli.data_dir).await,
        Cmd::Completions(a) => completions::run(a, cli.json, cli.data_dir).await,
        Cmd::Prune(a) => prune::run(a, cli.json, cli.data_dir).await,
        Cmd::Rebuild(a) => rebuild::run(a, cli.json, cli.data_dir).await,
        Cmd::Gc => gc::run(cli.json, cli.data_dir).await,
        Cmd::InstallHooks(a) => install_hooks::run(a, cli.json, cli.data_dir).await,
    }
}

/// Resolve the effective data directory. Caller passes the CLI flag (which
/// also reads `COMEMORY_DATA_DIR`); on `None` we fall back to `$HOME/.comemory`.
pub fn resolve_data_dir(opt: Option<std::path::PathBuf>) -> std::path::PathBuf {
    opt.unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        std::path::PathBuf::from(home).join(".comemory")
    })
}

/// Build the retrieval [`PageWindow`] for a paginated subcommand from its
/// `--k`/`--limit` page size (`None` → configured `retrieval.top_k`) and
/// `--offset`. Shared by `search`, `search-code`, and `context` so the
/// three subcommands cannot drift on what "page size" means (Binding
/// Rule 1). `--k 0` / `--limit 0` is preserved as the "all remaining
/// within the window" sentinel.
pub(crate) fn page_window(
    cfg: &Config,
    k: Option<usize>,
    offset: usize,
) -> crate::retrieval::pipeline::PageWindow {
    crate::retrieval::pipeline::PageWindow {
        offset,
        limit: k.unwrap_or(cfg.retrieval.top_k),
    }
}

/// Translate a finished pipeline run's window metadata into the
/// [`crate::output::search::PageMeta`] the JSON envelopes carry. Shared by
/// the three paginated subcommands so the cursor shape stays uniform.
pub(crate) fn page_meta(
    window: crate::retrieval::pipeline::PageWindow,
    has_more: bool,
    total: usize,
) -> crate::output::search::PageMeta {
    crate::output::search::PageMeta {
        limit: window.limit,
        offset: window.offset,
        has_more,
        total: Some(total),
    }
}

/// Split a comma-separated flag value into trimmed, non-empty, de-duplicated
/// entries preserving first-mention order. Shared by `save` (`--tags`,
/// `--supersedes`) and `feedback` (`--used`, `--irrelevant`) so every CSV
/// flag tolerates `a,,a , b` style input identically.
pub(crate) fn csv_unique(raw: &str) -> Vec<String> {
    if raw.is_empty() {
        return Vec::new();
    }
    let mut seen = std::collections::HashSet::new();
    raw.split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty() && seen.insert(t.clone()))
        .collect()
}

/// Parse a CSV of memory ids via [`csv_unique`] and validate every entry
/// against [`crate::memory::id::is_valid_memory_id`], naming the offending
/// `flag` in the error. Shared by `save --supersedes` and the `feedback`
/// id flags so malformed ids are rejected identically everywhere.
pub(crate) fn parse_id_csv(raw: &str, flag: &str) -> Result<Vec<String>> {
    let ids = csv_unique(raw);
    for entry in &ids {
        if !crate::memory::id::is_valid_memory_id(entry) {
            return Err(Error::Config(format!(
                "{flag}: invalid memory id `{entry}` (expected 8 lowercase hex chars)"
            )));
        }
    }
    Ok(ids)
}

/// Parse a CSV of code-symbol ids via [`csv_unique`] and validate every
/// entry as a positive integer (`code_symbols.id` is an INTEGER rowid; 0
/// and negatives never name a row), naming the offending `flag` in the
/// error. De-duplicates again on the parsed value so `07,7` cannot
/// double-count a counter. Sibling of [`parse_id_csv`] for the
/// `feedback --used-code` / `--irrelevant-code` flags.
pub(crate) fn parse_symbol_id_csv(raw: &str, flag: &str) -> Result<Vec<i64>> {
    let mut ids: Vec<i64> = Vec::new();
    for entry in csv_unique(raw) {
        let bad = || {
            Error::Config(format!(
                "{flag}: invalid symbol id `{entry}` (expected a positive integer)"
            ))
        };
        let id: i64 = entry.parse().map_err(|_| bad())?;
        if id <= 0 {
            return Err(bad());
        }
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    Ok(ids)
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
