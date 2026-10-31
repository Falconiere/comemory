//! `comemory run embedding`: explicit folder reconciliation and embedding.

use crate::cli::{load_config, output::json};
use crate::config::paths::{Paths, resolve_data_dir};
use crate::domains::documents::embedding;
use crate::prelude::*;
use crate::utilities::context::Ctx;
use clap::{Args as ClapArgs, Subcommand};
use std::io::Write as _;
use std::path::PathBuf;

const EXAMPLES: &str = "Examples:
  comemory run embedding
  comemory run embedding --path ~/Documents/notes
  comemory run embedding --path . --repo myrepo
  comemory run embedding --mode full";

/// Arguments for the explicit run command family.
#[derive(Debug, ClapArgs)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Explicit operation to execute.
    #[command(subcommand)]
    pub cmd: RunCmd,
}

/// Explicit operations.
#[derive(Debug, Subcommand)]
pub enum RunCmd {
    /// Reconcile supported documents and code in the selected folder.
    Embedding(EmbeddingArgs),
}

/// Folder selection and optional metadata for an embedding run.
#[derive(Debug, ClapArgs)]
#[command(after_help = EXAMPLES)]
pub struct EmbeddingArgs {
    /// Selected directory; defaults to the current directory, without widening.
    #[arg(long, default_value = ".")]
    pub path: PathBuf,
    /// Repository metadata; does not change which directory is scanned.
    #[arg(long)]
    pub repo: Option<String>,
    /// Incremental updates changed content; full refreshes supported derived data.
    #[arg(long, value_enum, default_value_t = crate::cli::index_code::Mode::Incremental)]
    pub mode: crate::cli::index_code::Mode,
}

/// Execute on a blocking thread, with one selected root and one report.
pub async fn run(args: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let RunCmd::Embedding(args) = args.cmd;
    let root = std::env::current_dir()?.join(args.path);
    let result = tokio::task::spawn_blocking(move || {
        let paths = Paths::new(resolve_data_dir(data_dir));
        paths.ensure_dirs()?;
        let cfg = load_config(&paths)?;
        let mut ctx = Ctx::lazy(&paths, &cfg);
        embedding::run(
            &mut ctx,
            embedding::Request {
                path: root.to_string_lossy().into_owned(),
                repo: args.repo,
                mode: args.mode.into(),
            },
        )
    })
    .await
    .map_err(|e| Error::Other(format!("embedding worker failed: {e}")))??;
    emit(json_flag, &result)
}

fn emit(json_flag: bool, result: &embedding::Response) -> Result<()> {
    if json_flag {
        return json::write(result);
    }
    let mut out = std::io::stdout().lock();
    for warning in &result.warnings {
        writeln!(out, "warning: {warning}")?;
    }
    for source in &result.documents.sources {
        writeln!(
            out,
            "documents {}: indexed={} unchanged={} removed={} embedded={} pending={} failed={} unsupported={}",
            source.canonical_path,
            source.indexed,
            source.unchanged,
            source.removed,
            source.embedded,
            source.embedding_pending,
            source.embedding_failed,
            source.unsupported
        )?;
        for error in &source.errors {
            writeln!(out, "warning: {}: {}", error.path, error.message)?;
        }
    }
    writeln!(
        out,
        "code {}: indexed={} unchanged={} removed={} unsupported={} embedded={} pending={} failed={}",
        result.code.repo,
        result.code.files_indexed,
        result.code.unchanged,
        result.code.removed,
        result.code.unsupported,
        result.code.embedded,
        result.code.embedding_pending,
        result.code.embedding_failed
    )?;
    Ok(())
}
