//! Clap-driven CLI surface for the memory layer. The dispatcher in `run`
//! delegates to one-file-per-subcommand modules so each command owns its own
//! argument shape and output rendering.

use clap::{Parser, Subcommand};

use crate::prelude::*;

pub mod ast;
pub mod completions;
pub mod conflicts;
pub mod context;
pub mod delete;
pub mod doctor;
pub mod feedback;
pub mod gc;
pub mod index_code;
pub mod install_hooks;
pub mod list;
pub mod memory_for;
pub mod prune;
pub mod save;
pub mod search;
pub mod supersedes;
pub mod symbol;
pub mod walk;

/// Top-level CLI. `qwick-memory <subcommand> [--json] [--data-dir DIR]`. The `--json`
/// and `--data-dir` flags are global so callers can place them either before
/// or after the subcommand.
#[derive(Parser, Debug)]
#[command(
    name = "qwick-memory",
    version,
    about = "Agentic dev memory + code-aware semantic search",
    arg_required_else_help = true
)]
pub struct Cli {
    /// Emit machine-readable JSON instead of a human TTY view.
    #[arg(long, global = true)]
    pub json: bool,

    /// Override the data root (defaults to `$HOME/.qwick-memory`). Honors the
    /// `QWICK_MEMORY_DATA_DIR` environment variable.
    #[arg(long, global = true, env = "QWICK_MEMORY_DATA_DIR")]
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
    /// List memories with optional repo/kind filters.
    List(list::Args),
    /// Soft-delete a memory by id (moves to `.trash/`).
    Delete(delete::Args),
    /// Record per-memory feedback (used vs irrelevant).
    Feedback(feedback::Args),
    /// Report on the data directory and memory count.
    Doctor,
    /// Walk a repo, extract symbols, and upsert into the code index.
    IndexCode(index_code::Args),
    /// Semantic search over the code index for a symbol name.
    Symbol(symbol::Args),
    /// List memories that reference a qualified symbol or file path.
    MemoryFor(memory_for::Args),
    /// Run an ast-grep pattern against a single source file.
    Ast(ast::Args),
    /// Headline lookup: code symbol + memories matching a key.
    Context(context::Args),
    /// Walk a graph edge from a memory id (currently `--edge supersedes`).
    Walk(walk::Args),
    /// Emit a shell completion script for `bash`, `zsh`, `fish`, `powershell`, or `elvish`.
    Completions(completions::Args),
    /// List memories that conflict with the given memory id.
    Conflicts(conflicts::Args),
    /// Record that one memory supersedes another in the kuzu graph.
    Supersedes(supersedes::Args),
    /// Detect (and optionally soft-delete) stale memories.
    Prune(prune::Args),
    /// Purge old entries from `memories/.trash/`.
    Gc,
    /// Install git hooks that trigger `qwick-memory index-code --incremental` on
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
        Cmd::List(a) => list::run(a, cli.json, cli.data_dir).await,
        Cmd::Delete(a) => delete::run(a, cli.json, cli.data_dir).await,
        Cmd::Feedback(a) => feedback::run(a, cli.json, cli.data_dir).await,
        Cmd::Doctor => doctor::run(cli.json, cli.data_dir).await,
        Cmd::IndexCode(a) => index_code::run(a, cli.json, cli.data_dir).await,
        Cmd::Symbol(a) => symbol::run(a, cli.json, cli.data_dir).await,
        Cmd::MemoryFor(a) => memory_for::run(a, cli.json, cli.data_dir).await,
        Cmd::Ast(a) => ast::run(a, cli.json, cli.data_dir).await,
        Cmd::Context(a) => context::run(a, cli.json, cli.data_dir).await,
        Cmd::Walk(a) => walk::run(a, cli.json, cli.data_dir).await,
        Cmd::Completions(a) => completions::run(a, cli.json, cli.data_dir).await,
        Cmd::Conflicts(a) => conflicts::run(a, cli.json, cli.data_dir).await,
        Cmd::Supersedes(a) => supersedes::run(a, cli.json, cli.data_dir).await,
        Cmd::Prune(a) => prune::run(a, cli.json, cli.data_dir).await,
        Cmd::Gc => gc::run(cli.json, cli.data_dir).await,
        Cmd::InstallHooks(a) => install_hooks::run(a, cli.json, cli.data_dir).await,
    }
}

/// Resolve the effective data directory. Caller passes the CLI flag (which
/// also reads `QWICK_MEMORY_DATA_DIR`); on `None` we fall back to `$HOME/.qwick-memory`.
pub fn resolve_data_dir(opt: Option<std::path::PathBuf>) -> std::path::PathBuf {
    opt.unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        std::path::PathBuf::from(home).join(".qwick-memory")
    })
}
