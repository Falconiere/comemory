//! `comemory index-code` — incremental symbol extraction over a real git
//! repo, mirrored into the `code_symbols` SQLite table via the shared
//! [`api::index_code`] middle (Binding Rule 1).
//!
//! `--extract` stays CLI-only: it emits one JSONL row per symbol on
//! stdout instead of writing to the DB, skipping `connection::open`
//! entirely so a read-only data dir is not a blocker. Callers feed
//! `comemory ingest-code` with pre-embedded JSONL when they want a
//! `code_vec` row.

use std::io::Write as _;
use std::path::PathBuf;

use clap::{Args as ClapArgs, ValueEnum};
use git2::Repository;
use ignore::WalkBuilder;

use crate::api::index_code::walk::{
    blob_oid, chunk_symbol, parent_snippet_of, relative, simhash_of,
};
use crate::api::{self, Ctx};
use crate::ast::extractor::ExtractedSymbol;
use crate::ast::{self, languages};
use crate::cli::load_config;
use crate::config::paths::{Paths, resolve_data_dir};
use crate::git_utils::map_git_err;
use crate::prelude::*;

const EXAMPLES: &str = "\
Examples:
  # Index the current working directory with explicit repo label
  comemory index-code --repo myrepo --path .

  # Re-extract every file, not just the ones whose blob changed (drops the
  # repo's BYO code vectors — re-run `comemory ingest-code` afterwards)
  comemory index-code --repo myrepo --path . --mode full

  # Emit one JSONL row per symbol on stdout (skips DB writes)
  comemory index-code --repo myrepo --path ./src --extract";

/// Arguments to `comemory index-code`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Repo label stored alongside each symbol row.
    #[arg(long)]
    pub repo: String,
    /// Root of the working tree to walk. Must live inside a git repo so
    /// blob OIDs are available for the incremental skip path.
    #[arg(long)]
    pub path: PathBuf,
    /// Emit JSONL on stdout instead of inserting rows. Suitable for piping
    /// into an external embedder + `comemory ingest-code`.
    #[arg(long, default_value_t = false)]
    pub extract: bool,
    /// `incremental` (default) re-extracts only files whose blob OID moved
    /// since the last run; `full` clears the repo's indexed-file cursor first
    /// so every file re-extracts. `full` is lossy: re-extracting a file
    /// replaces its symbol rows, which drops the repo's BYO code vectors
    /// (`code_vec`) and resets per-symbol access counters — re-run
    /// `ingest-code` afterwards to restore the semantic leg.
    #[arg(long, value_enum, default_value_t = Mode::Incremental)]
    pub mode: Mode,
}

/// `--mode` values, mirrored onto [`api::index_code::IndexMode`] (the CLI
/// keeps its `ValueEnum` derive, the HTTP surface a plain `Deserialize`).
#[derive(Copy, Clone, Debug, ValueEnum)]
#[clap(rename_all = "lowercase")]
pub enum Mode {
    /// Only files changed since the last run.
    Incremental,
    /// Every file. Lossy: re-extraction replaces each file's symbol rows,
    /// which drops the repo's BYO `code_vec` rows and resets its per-symbol
    /// access counters — re-run `ingest-code` afterwards to restore the
    /// semantic leg.
    Full,
}

impl From<Mode> for api::index_code::IndexMode {
    fn from(m: Mode) -> Self {
        match m {
            Mode::Incremental => Self::Incremental,
            Mode::Full => Self::Full,
        }
    }
}

/// `--extract` streams JSONL to stdout; otherwise delegates the DB-write
/// walk to [`api::index_code::run`].
pub async fn run(args: Args, _json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    if args.extract {
        let repo = Repository::open(&args.path).map_err(map_git_err)?;
        return run_extract(&args, &repo);
    }
    let cfg = load_config(&paths)?;
    let mut ctx = Ctx::lazy(&paths, &cfg);
    api::index_code::run(
        &mut ctx,
        api::index_code::Request {
            repo: args.repo.clone(),
            path: args.path.to_string_lossy().into_owned(),
            mode: args.mode.into(),
        },
    )?;
    Ok(())
}

/// `--extract` path. Walks the same files as the DB-write path but emits
/// every symbol as a JSONL row on stdout without opening a SQLite
/// connection. The `indexed_files` cursor is *not* consulted under
/// `--extract` so callers can re-feed an embedder deterministically over
/// an unchanged repo.
fn run_extract(args: &Args, repo: &Repository) -> Result<()> {
    let mut walker = WalkBuilder::new(&args.path);
    walker.standard_filters(true);
    for entry in walker.build().filter_map(std::result::Result::ok) {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Some(lang) = languages::detect(entry.path()) else {
            continue;
        };
        let rel = relative(&args.path, entry.path());
        let Some(oid) = blob_oid(repo, entry.path()) else {
            continue;
        };
        let snippet = std::fs::read_to_string(entry.path()).map_err(Error::Io)?;
        let symbols = ast::extract(lang, &snippet)?;
        for s in &symbols {
            emit_symbol_jsonl(&args.repo, &rel, &oid, lang, s)?;
        }
    }
    Ok(())
}

/// Serialise a single extracted symbol as JSONL on stdout. JSONL contract
/// (consumed by `comemory ingest-code`): unchunked symbols emit exactly
/// one row. cAST-chunked symbols emit the parent row first (headline
/// snippet, no `parent_symbol` field) followed by one row per chunk
/// carrying two extra fields — `parent_symbol` (the parent's `symbol`
/// value) and `chunk_index` (one-based).
fn emit_symbol_jsonl(
    repo: &str,
    rel: &str,
    blob_oid: &str,
    lang: languages::Lang,
    s: &ExtractedSymbol,
) -> Result<()> {
    let snippet = parent_snippet_of(s);
    let mut out = std::io::stdout().lock();
    let row = serde_json::json!({
        "repo": repo,
        "path": rel,
        "blob_oid": blob_oid,
        "symbol": s.name,
        "kind": s.kind,
        "lang": lang.as_str(),
        "line_start": s.line as i64,
        "line_end": s.line_end as i64,
        "snippet": snippet,
        "simhash": simhash_of(&snippet),
    });
    writeln!(out, "{row}").map_err(Error::Io)?;
    for (i, c) in s.chunks.iter().enumerate() {
        let row = serde_json::json!({
            "repo": repo,
            "path": rel,
            "blob_oid": blob_oid,
            "symbol": chunk_symbol(&s.name, i),
            "kind": s.kind,
            "lang": lang.as_str(),
            "line_start": c.line_start as i64,
            "line_end": c.line_end as i64,
            "snippet": c.text,
            "simhash": simhash_of(&c.text),
            "parent_symbol": s.name,
            "chunk_index": (i + 1) as i64,
        });
        writeln!(out, "{row}").map_err(Error::Io)?;
    }
    Ok(())
}
