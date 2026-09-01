//! `comemory save` — atomic markdown write + SQLite-mirror upsert.
//!
//! Clap `Args` parsing plus the stdin body read (the only place stdin
//! exists — spec §Architecture) live here. The raw `--vector`/
//! `--vector-stdin` flags pass through unparsed to
//! [`crate::api::save::run`] (Binding Rule 1, shared with `POST
//! /api/v1/memories`), which parses (and reads stdin for) them AFTER its
//! own `supersedes`/`ref_*` validation — see that function's doc.

use std::io::Read;
use std::io::Write as _;
use std::path::PathBuf;

use clap::Args as ClapArgs;

use crate::api;
use crate::cli::{csv_unique, load_config};
use crate::config::paths::{Paths, resolve_data_dir};
use crate::memory::Kind;
use crate::output::tty;
use crate::prelude::*;

const EXAMPLES: &str = "\
Examples:
  # Save a decision with tags and elevated quality
  comemory save \"Use Postgres for analytics\" --kind decision --repo myrepo --tags db,postgres --quality 4

  # Pipe a bug report body from another command
  echo \"Race in run_migration when run twice in <1s\" | comemory save - --kind bug --repo myrepo

  # Save with a caller-supplied embedding (BYO-vector)
  echo '{\"embedding\":[0.1,0.2,...]}' | comemory save \"...body...\" --vector-stdin

  # Minimal note (kind defaults to `note`, no repo/tags)
  comemory save \"Remember: cargo nextest serializes the embedder group\"

  # Replace an outdated memory: a1b2c3d4 is annotated `superseded_by` in
  # search results and demoted in ranking (score_parts.supersede = 0.2)
  comemory save \"new convention: pgbouncer in transaction mode\" --supersedes a1b2c3d4

  # Near-duplicate detection: if a similar memory exists, a TTY warning is
  # printed to stderr and --json output includes a `duplicate_of` field with
  # the matching memory id. The save always proceeds — use `--supersedes` to
  # mark the relationship if the new memory replaces the old one.";

/// Arguments to `comemory save`. The positional `body` is optional — if omitted
/// or `-`, the body is read from stdin so callers can pipe content.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// Memory body. Use `-` (or omit) to read from stdin.
    pub body: Option<String>,
    /// Memory kind: decision|bug|convention|discovery|pattern|note.
    #[arg(long, value_enum, default_value_t = Kind::Note)]
    pub kind: Kind,
    /// Repo name attached to the memory (free-form string).
    #[arg(long, default_value = "")]
    pub repo: String,
    /// Comma-separated tag list (e.g. `database,postgres`).
    #[arg(long, default_value = "")]
    pub tags: String,
    /// Author identifier. Defaults to empty so callers may omit.
    #[arg(long, default_value = "")]
    pub author: String,
    /// Quality rating 1..=5. Defaults to 3.
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u8).range(1..=5))]
    pub quality: u8,
    /// Comma-separated 8-hex memory ids this memory replaces (e.g.
    /// `a1b2c3d4,e5f6a7b8`). Recorded in the frontmatter
    /// `relations.supersedes` list and materialized as `supersedes` edges,
    /// so the older memories are demoted in ranking and annotated
    /// `superseded_by` in search results.
    #[arg(long, default_value = "")]
    pub supersedes: String,
    /// Caller-supplied dense vector as a comma-separated float list. Length
    /// must equal the configured memory vector dim or the save fails with
    /// `vector dim mismatch`.
    #[arg(long)]
    pub vector: Option<String>,
    /// Read a JSON `{ "embedding": [..] }` payload from stdin and use it as
    /// the dense vector for the saved memory. Mutually exclusive with body
    /// being read from stdin (the body must be supplied as a positional arg
    /// when `--vector-stdin` is set).
    #[arg(long, default_value_t = false)]
    pub vector_stdin: bool,
    /// Version-anchored file reference `[repo:]path` (repeatable;
    /// comma-splittable). Pins the HEAD-tree blob + commit + branch when the
    /// path is tracked in the cwd repo; untracked/cross-repo refs save
    /// unpinned with an advisory warning.
    #[arg(long)]
    pub ref_file: Vec<String>,
    /// Version-anchored symbol reference `[repo:]path:symbol` (repeatable;
    /// comma-splittable). A value without a trailing `:symbol` is a usage
    /// error (exit 64). Anchoring matches `--ref-file`.
    #[arg(long)]
    pub ref_symbol: Vec<String>,
}

/// Save the body and emit the new memory id + on-disk path.
///
/// Uses a lazy `Ctx` (no data-dir/DB touch up front) and passes the raw
/// `--vector`/`--vector-stdin` flags straight through to `api::save::run`
/// unparsed, so its `supersedes`/`ref_*` validation runs (and can fail)
/// before the vector is parsed, the data dir is created, the DB is opened,
/// or stdin is read — exactly as `cli::save::run` did pre-extraction
/// (AC-13).
pub async fn run(a: Args, json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let body = read_body(&a)?;

    let paths = Paths::new(resolve_data_dir(data_dir));
    let cfg = load_config(&paths)?;
    let mut ctx = api::Ctx::lazy(&paths, &cfg);

    let req = api::save::Request {
        body,
        title: None,
        kind: a.kind,
        repo: a.repo,
        tags: csv_unique(&a.tags),
        author: a.author,
        quality: a.quality,
        supersedes: csv_unique(&a.supersedes),
        vector: None,
        ref_file: a.ref_file,
        ref_symbol: a.ref_symbol,
    };
    let output = api::save::run(&mut ctx, req, a.vector_stdin, a.vector.as_deref())?;
    emit(json, &output)
}

/// Resolve the body from the positional arg or stdin, rejecting the
/// `--vector-stdin` + stdin-body combination (both would consume stdin).
fn read_body(a: &Args) -> Result<String> {
    match a.body.as_deref() {
        Some("-") | None => {
            if a.vector_stdin {
                return Err(Error::Config(
                    "--vector-stdin requires the body to be passed as a positional arg".into(),
                ));
            }
            read_stdin()
        }
        Some(s) => Ok(s.to_string()),
    }
}

/// Emit the save result: a single JSON object under `--json`, else a TTY
/// summary with the near-dup advisory and each ref warning on stderr.
fn emit(json: bool, output: &api::save::Response) -> Result<()> {
    let mut out = std::io::stdout().lock();
    if json {
        writeln!(out, "{}", serde_json::to_string(output)?)?;
        return Ok(());
    }
    writeln!(out, "saved {}", output.id)?;
    writeln!(out, "  path: {}", output.path)?;
    if let Some(dup) = output.duplicate_of.as_deref() {
        tty::warning(&format!(
            "similar memory {dup} exists — consider supersedes"
        ))?;
    }
    for w in &output.warnings {
        tty::warning(w)?;
    }
    Ok(())
}

fn read_stdin() -> Result<String> {
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    Ok(buf)
}
