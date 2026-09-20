//! `comemory architecture` — scaffold, save, show, check and learn the
//! component-level architecture model of an indexed repository (CLI-only).
//!
//! Every subcommand is a thin shell around `domains::architecture`: this file
//! owns flags, repo resolution, stdin/file reading and the `--format` switch,
//! and nothing else. The global `--json` flag wins over `--format`, matching
//! `comemory graph`.

use std::io::Read as _;
use std::path::PathBuf;

use clap::{Args as ClapArgs, Subcommand, ValueEnum};

use crate::cli::load_config;
use crate::cli::output::{architecture as render, json};
use crate::config::paths::{Paths, resolve_data_dir};
use crate::domains::architecture::{check, current, learn, model::Model, save, scaffold};
use crate::domains::code::git_utils::repo_label_at;
use crate::prelude::*;
use crate::store::connection;
use crate::utilities::context::Ctx;

const EXAMPLES: &str = "\
Examples:
  # Deterministic scaffold of the repo in the current directory
  comemory architecture scaffold --json > model.json

  # Store an enriched model (validated against the code index)
  comemory architecture save model.json

  # Draw it
  comemory architecture show --format mermaid

  # What has the model stopped describing?
  comemory architecture check --json

  # Let an agent enrich the scaffold, then store the result
  comemory architecture learn --command 'claude -p \"$(cat {prompt_file})\"'";

/// Top-level `architecture` args — nested subcommand required.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Nested action.
    #[command(subcommand)]
    pub cmd: ArchitectureCmd,
}

/// Nested architecture subcommands.
#[derive(Subcommand, Debug)]
pub enum ArchitectureCmd {
    /// Emit a deterministic model scaffolded from the indexed code graph.
    Scaffold(ScaffoldArgs),
    /// Validate a model and store it as this repo's architecture memory.
    Save(SaveArgs),
    /// Print the stored model as JSON or Mermaid.
    Show(ShowArgs),
    /// Report drift between the stored model and today's index.
    Check(CheckArgs),
    /// Hand the scaffold to an agent command and store what it prints.
    Learn(LearnArgs),
}

/// Scaffold knobs shared by `scaffold`, `check` and `learn`.
#[derive(ClapArgs, Debug)]
pub struct ShapeArgs {
    /// Directory-prefix depth a component clusters at. Must be >= 1.
    #[arg(
        long,
        default_value_t = 2,
        value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..)
    )]
    pub depth: usize,
    /// Keep at most this many components, highest rank first. Must be >= 1.
    #[arg(
        long,
        default_value_t = 120,
        value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..)
    )]
    pub max_components: usize,
    /// Drop component edges below this weight. Must be >= 1.
    #[arg(
        long,
        default_value_t = 1,
        value_parser = clap::builder::RangedI64ValueParser::<i64>::new().range(1..)
    )]
    pub min_edge_weight: i64,
}

impl From<&ShapeArgs> for scaffold::Options {
    fn from(a: &ShapeArgs) -> Self {
        Self {
            depth: a.depth,
            max_components: a.max_components,
            min_edge_weight: a.min_edge_weight,
        }
    }
}

/// Args for `architecture scaffold`.
#[derive(ClapArgs, Debug)]
pub struct ScaffoldArgs {
    /// Repo label (defaults to the current directory's repository).
    #[arg(long)]
    pub repo: Option<String>,
    /// Clustering knobs.
    #[command(flatten)]
    pub shape: ShapeArgs,
}

/// Args for `architecture save`.
#[derive(ClapArgs, Debug)]
pub struct SaveArgs {
    /// Model file, or `-` for stdin (the default).
    #[arg(default_value = "-")]
    pub file: String,
    /// Repo label (defaults to the current directory's repository).
    #[arg(long)]
    pub repo: Option<String>,
}

/// Output shape for `architecture show`.
#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum Format {
    /// The stored model as JSON.
    Json,
    /// Mermaid `flowchart` source.
    Mermaid,
}

/// Args for `architecture show`.
#[derive(ClapArgs, Debug)]
pub struct ShowArgs {
    /// Repo label (defaults to the current directory's repository).
    #[arg(long)]
    pub repo: Option<String>,
    /// Output format. The global `--json` flag overrides it.
    #[arg(long, value_enum, default_value_t = Format::Json)]
    pub format: Format,
}

/// Args for `architecture check`.
#[derive(ClapArgs, Debug)]
pub struct CheckArgs {
    /// Repo label (defaults to the current directory's repository).
    #[arg(long)]
    pub repo: Option<String>,
    /// Clustering knobs for the comparison scaffold.
    #[command(flatten)]
    pub shape: ShapeArgs,
}

/// Args for `architecture learn`.
#[derive(ClapArgs, Debug)]
pub struct LearnArgs {
    /// Agent command template; must contain `{prompt_file}` or `{prompt}`.
    #[arg(long)]
    pub command: String,
    /// Repo label (defaults to the current directory's repository).
    #[arg(long)]
    pub repo: Option<String>,
    /// Kill the agent command after this many seconds.
    #[arg(long, default_value_t = 600)]
    pub timeout: u64,
    /// Write the prompt and stop, without starting the agent.
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
    /// Clustering knobs for the scaffold handed to the agent.
    #[command(flatten)]
    pub shape: ShapeArgs,
}

/// Dispatch the nested architecture subcommands.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let mut conn = connection::open(paths.db_path())?;
    let cfg = load_config(&paths)?;
    let mut ctx = Ctx::borrowed(&paths, &cfg, &mut conn);

    match a.cmd {
        ArchitectureCmd::Scaffold(s) => {
            let repo = resolve_repo(s.repo)?;
            let model = scaffold::run(ctx.conn()?, &repo, &(&s.shape).into())?;
            if json_flag {
                json::write(&model)
            } else {
                render::write_model(&model)
            }
        }
        ArchitectureCmd::Save(s) => {
            let repo = resolve_repo(s.repo)?;
            let model = read_model(&s.file)?;
            let saved = save::run(&mut ctx, &repo, &model)?;
            if json_flag {
                json::write(&saved)
            } else {
                render::write_saved(&saved)
            }
        }
        ArchitectureCmd::Show(s) => {
            let repo = resolve_repo(s.repo)?;
            let stored = current::require(ctx.conn()?, &repo)?;
            emit_model(&stored.model, json_flag, s.format)
        }
        ArchitectureCmd::Check(c) => {
            let repo = resolve_repo(c.repo)?;
            let drift = check::run(ctx.conn()?, &repo, &(&c.shape).into())?;
            if json_flag {
                json::write(&drift)
            } else {
                render::write_drift(&drift)
            }
        }
        ArchitectureCmd::Learn(l) => {
            let repo = resolve_repo(l.repo)?;
            let opts = learn::Options {
                command: l.command,
                timeout_secs: l.timeout,
                dry_run: l.dry_run,
                scaffold: (&l.shape).into(),
            };
            let learned = learn::run(&mut ctx, &repo, &opts).await?;
            if json_flag {
                json::write(&learned)
            } else {
                render::write_learned(&learned)
            }
        }
    }
}

/// Emit a stored model for `show`, with the global `--json` flag overriding
/// `--format` exactly as it does for `comemory graph --format dot --json`.
fn emit_model(model: &Model, json_flag: bool, format: Format) -> Result<()> {
    match (json_flag, format) {
        (true, _) | (false, Format::Json) => json::write(model),
        (false, Format::Mermaid) => render::write_mermaid(model),
    }
}

/// The repo label to work on: the flag, else the current directory's git
/// repository.
fn resolve_repo(flag: Option<String>) -> Result<String> {
    if let Some(repo) = flag {
        return Ok(repo);
    }
    let cwd = std::env::current_dir()?;
    repo_label_at(&cwd).ok_or_else(|| {
        Error::Usage(format!(
            "{} is not inside a git repository; pass --repo",
            cwd.display()
        ))
    })
}

/// Read a model from `file`, or from stdin when it is `-`.
fn read_model(file: &str) -> Result<Model> {
    let raw = if file == "-" {
        let mut buf = String::new();
        std::io::stdin().lock().read_to_string(&mut buf)?;
        buf
    } else {
        std::fs::read_to_string(file)?
    };
    Ok(serde_json::from_str(&raw)?)
}
