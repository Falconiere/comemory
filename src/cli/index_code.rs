//! `comemory index-code` — incremental symbol extraction over a real git
//! repo, mirrored into the `code_symbols` SQLite table.
//!
//! v0.2 collapses the v0.1 LanceDB write into a single transaction against
//! `comemory.db`: per-file blob OIDs gate the work (so re-runs are
//! O(touched-files), not O(repo)), and per-symbol rows land in
//! `code_symbols` + `code_fts`. Vectors are intentionally not produced here —
//! callers feed `comemory ingest-code` with pre-embedded JSONL when they
//! want a `code_vec` row, or pass `--extract` to emit JSONL on stdout for
//! piping into an external embedder.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use clap::Args as ClapArgs;
use git2::Repository;
use ignore::WalkBuilder;
use rusqlite::Connection;

use crate::ast::extractor::ExtractedSymbol;
use crate::ast::{self, languages};
use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::git_utils::map_git_err;
use crate::prelude::*;
use crate::simhash;
use crate::store::code_row::{self, CodeSymbolRow};
use crate::store::{connection, fts};

const EXAMPLES: &str = "\
Examples:
  # Index the current working directory with explicit repo label
  comemory index-code --repo myrepo --path .

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
}

/// Walk `args.path`, extract symbols from every supported source file, and
/// mirror them into `code_symbols` (+ `code_fts`) — or emit them as JSONL
/// when `--extract` is set.
///
/// When the DB-write path is taken (i.e. `--extract` is not set), the whole
/// walk runs inside a single SQLite transaction so a mid-walk failure rolls
/// back cleanly: no partial `code_symbols`/`code_fts` rows, no stale
/// `indexed_files` cursors, and re-running `index-code` on the same blob does
/// not produce duplicate symbol rows for files whose `indexed_files` row had
/// not yet been written.
pub async fn run(args: Args, _json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let mut conn = connection::open(paths.db_path())?;
    let repo = Repository::open(&args.path).map_err(map_git_err)?;

    let tx = conn.transaction()?;
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
        let oid = match blob_oid(&repo, entry.path()) {
            Some(v) => v,
            // Untracked files (or files whose canonicalisation failed — that
            // case is logged inside `blob_oid`) have no blob OID we can use as
            // an indexing cursor, so skip them.
            None => continue,
        };
        if oid_is_indexed(&tx, &args.repo, &rel, &oid)? {
            continue;
        }
        // Drop any prior `code_symbols`/`code_vec`/`code_fts` rows for this
        // `(repo, path)` before re-inserting so a blob_oid change (file edited)
        // does not leave stale symbol rows behind alongside the fresh ones, and
        // so re-inserts with shifted `line_start` values cannot collide on the
        // `UNIQUE(repo, path, symbol, line_start)` constraint mid-transaction.
        if !args.extract {
            code_row::purge_file_symbols(&tx, &args.repo, &rel)?;
        }
        let snippet = std::fs::read_to_string(entry.path()).map_err(Error::Io)?;
        let symbols = ast::extract(lang, &snippet)?;
        for s in &symbols {
            handle_symbol(&tx, args.extract, &args.repo, &rel, &oid, lang, s)?;
        }
        code_row::upsert_indexed_file(&tx, &args.repo, &rel, &oid)?;
    }
    tx.commit()?;
    Ok(())
}

/// Insert (or print, under `--extract`) a single extracted symbol row.
fn handle_symbol(
    conn: &Connection,
    extract_mode: bool,
    repo: &str,
    rel: &str,
    blob_oid: &str,
    lang: languages::Lang,
    s: &ExtractedSymbol,
) -> Result<()> {
    let line_start = s.line as i64;
    let line_end = line_end_of(s) as i64;
    let token_iter = simhash::tokens(&s.snippet);
    let sh = simhash::simhash64(token_iter.iter().map(|t| t.as_str())) as i64;

    if extract_mode {
        let row = serde_json::json!({
            "repo": repo,
            "path": rel,
            "blob_oid": blob_oid,
            "symbol": s.name,
            "kind": s.kind,
            "lang": lang.as_str(),
            "line_start": line_start,
            "line_end": line_end,
            "snippet": s.snippet,
            "simhash": sh,
        });
        let mut out = std::io::stdout().lock();
        writeln!(out, "{row}").map_err(Error::Io)?;
        return Ok(());
    }

    let sid = code_row::insert(
        conn,
        &CodeSymbolRow {
            repo,
            path: rel,
            blob_oid,
            symbol: &s.name,
            kind: &s.kind,
            lang: lang.as_str(),
            line_start,
            line_end,
            snippet: &s.snippet,
            simhash: sh,
        },
    )?;
    fts::index_code(conn, sid, &s.name, &s.snippet, &fts::path_to_tokens(rel))?;
    Ok(())
}

/// Returns true when the `indexed_files` table already records `oid` for
/// `repo + path`, meaning the working-tree blob hasn't changed since the
/// last `index-code` run.
fn oid_is_indexed(conn: &Connection, repo: &str, path: &str, oid: &str) -> Result<bool> {
    let row: Option<String> = conn
        .query_row(
            "SELECT blob_oid FROM indexed_files WHERE repo = ?1 AND path = ?2",
            rusqlite::params![repo, path],
            |r| r.get(0),
        )
        .ok();
    Ok(matches!(row, Some(v) if v == oid))
}

/// Look up the working-tree blob OID for `file` via the repo index. Files
/// that are untracked (or unreadable through git) return `None` so the caller
/// can skip them.
///
/// `libgit2`'s `Index::get_path` rejects absolute paths, so we canonicalize
/// both the working tree and the file path before stripping the prefix — on
/// macOS the tempdir prefix differs (`/var/...` vs `/private/var/...`) so a
/// naive `strip_prefix` over the configured workdir would silently fall back
/// to passing the absolute path, which then panics inside libgit2.
///
/// Canonicalisation failures (e.g. a symlink with a broken target) are logged
/// via `tracing::warn!` so they don't masquerade as a plain untracked-file
/// skip — the caller still receives `None` and moves on, but the operator
/// sees the failed path in the logs.
fn blob_oid(repo: &Repository, file: &Path) -> Option<String> {
    let workdir = repo.workdir()?;
    let workdir_canon = match workdir.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(
                workdir = %workdir.display(),
                error = %e,
                "index-code: failed to canonicalise git workdir; skipping file",
            );
            return None;
        }
    };
    let file_canon = match file.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(
                file = %file.display(),
                error = %e,
                "index-code: failed to canonicalise file path; skipping",
            );
            return None;
        }
    };
    let rel = file_canon.strip_prefix(&workdir_canon).ok()?;
    let idx = repo.index().ok()?;
    let entry = idx.get_path(rel, 0)?;
    Some(entry.id.to_string())
}

/// Compute the inclusive last line of `s.snippet` relative to `s.line`.
/// `ExtractedSymbol::line` is one-based; for a single-line snippet this
/// returns `s.line` unchanged. Empty snippets fall back to `s.line` too.
fn line_end_of(s: &ExtractedSymbol) -> usize {
    let lines = s.snippet.lines().count();
    if lines <= 1 {
        s.line
    } else {
        s.line + lines - 1
    }
}

/// Render `file` relative to `root` for storage in the `code_symbols.path`
/// column. Falls back to the absolute path when `strip_prefix` fails so we
/// never silently store an empty string.
fn relative(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .unwrap_or(file)
        .to_string_lossy()
        .to_string()
}
