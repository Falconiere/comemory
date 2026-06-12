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

use std::borrow::Cow;
use std::collections::BTreeMap;
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
use crate::graph::{imports, materialize};
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
///
/// After the symbol transaction commits, a best-effort graph post-pass
/// ([`materialize::materialize`]) mines co-change pairs, refreshes the
/// import edges of the files (re)indexed this run, and projects PageRank
/// onto `code_symbols.rank_score`. Its errors are logged via
/// `tracing::warn!` and swallowed — the symbol index must land even when
/// graph materialization cannot. Files skipped by the blob-OID gate are
/// absent from `imports_by_file`, so their existing import edges survive
/// untouched (unchanged file, unchanged imports).
pub async fn run(args: Args, _json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let repo = Repository::open(&args.path).map_err(map_git_err)?;

    // `--extract` only emits JSONL to stdout, never writes to the DB.
    // Skip `connection::open` entirely so a read-only data dir is not a
    // blocker (and so we don't pay WAL setup for a no-op transaction).
    if args.extract {
        return run_extract(&args, &repo);
    }

    let mut conn = connection::open(paths.db_path())?;
    let mut imports_by_file: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let tx = conn.transaction()?;
    code_row::ensure_repo_format(&tx, &args.repo)?;
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
        code_row::purge_file_symbols(&tx, &args.repo, &rel)?;
        let snippet = std::fs::read_to_string(entry.path()).map_err(Error::Io)?;
        let symbols = ast::extract(lang, &snippet)?;
        for s in &symbols {
            write_symbol(&tx, &args.repo, &rel, &oid, lang, s)?;
        }
        // Collect this file's raw imports for the graph post-pass. An empty
        // Vec still matters — it clears the file's stale `imports` edges.
        // Extraction failures (an ast-grep pattern bug) must not sink the
        // symbol walk: warn and leave the file's previous edges in place.
        match imports::extract_imports(lang, &snippet) {
            Ok(modules) => {
                imports_by_file.insert(rel.clone(), modules);
            }
            Err(e) => {
                tracing::warn!(file = %rel, error = %e, "index-code: import extraction failed");
            }
        }
        code_row::upsert_indexed_file(&tx, &args.repo, &rel, &oid)?;
    }
    code_row::stamp_repo_format(&tx, &args.repo)?;
    tx.commit()?;
    // Best-effort graph post-pass: the symbol index above is already
    // durable; a graph failure (e.g. unborn HEAD) costs only freshness.
    if let Err(e) = materialize::materialize(&mut conn, &args.path, &args.repo, &imports_by_file) {
        tracing::warn!(
            repo = %args.repo,
            error = %e,
            "index-code: graph materialization failed; symbol index kept",
        );
    }
    Ok(())
}

/// `--extract` path. Walks the same files as the DB-write path but emits
/// every symbol as a JSONL row on stdout without opening a SQLite
/// connection. The caller's embedder + `comemory ingest-code` is expected
/// to persist the resulting rows. The `indexed_files` cursor is *not*
/// consulted under `--extract` so callers can re-feed an embedder
/// deterministically over an unchanged repo.
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
        let oid = match blob_oid(repo, entry.path()) {
            Some(v) => v,
            None => continue,
        };
        let snippet = std::fs::read_to_string(entry.path()).map_err(Error::Io)?;
        let symbols = ast::extract(lang, &snippet)?;
        for s in &symbols {
            emit_symbol_jsonl(&args.repo, &rel, &oid, lang, s)?;
        }
    }
    Ok(())
}

/// Insert a single extracted symbol into `code_symbols` + `code_fts`.
/// Used by the DB-write path only; the `--extract` path goes through
/// [`emit_symbol_jsonl`].
///
/// Unchunked symbols land as one row carrying the full snippet. cAST-
/// chunked symbols land as one PARENT row — its snippet reduced to the
/// headline (see [`parent_snippet_of`]) — followed by one CHILD row per
/// chunk: symbol `<name>#<n>`, the chunk's own line span / text /
/// simhash, and `parent_id` pointing at the parent's rowid. Each child
/// gets its own `code_fts` row; the parent keeps an fts row over the
/// headline. The `UNIQUE(repo, path, symbol, line_start)` constraint is
/// satisfied because `<name>#<n>` symbols are distinct from the parent's.
fn write_symbol(
    conn: &Connection,
    repo: &str,
    rel: &str,
    blob_oid: &str,
    lang: languages::Lang,
    s: &ExtractedSymbol,
) -> Result<()> {
    let snippet = parent_snippet_of(s);
    let parent_sid = code_row::insert(
        conn,
        &CodeSymbolRow {
            repo,
            path: rel,
            blob_oid,
            symbol: &s.name,
            kind: &s.kind,
            lang: lang.as_str(),
            line_start: s.line as i64,
            line_end: s.line_end as i64,
            snippet: &snippet,
            simhash: simhash_of(&snippet),
            parent_id: None,
        },
    )?;
    // The raw relative path goes straight into `code_fts.path_tokens`:
    // the identifier tokenizer handles the splitting (see fts::index_code).
    fts::index_code(conn, parent_sid, &s.name, &snippet, rel)?;
    for (i, c) in s.chunks.iter().enumerate() {
        let child_symbol = chunk_symbol(&s.name, i);
        let child_sid = code_row::insert(
            conn,
            &CodeSymbolRow {
                repo,
                path: rel,
                blob_oid,
                symbol: &child_symbol,
                kind: &s.kind,
                lang: lang.as_str(),
                line_start: c.line_start as i64,
                line_end: c.line_end as i64,
                snippet: &c.text,
                simhash: simhash_of(&c.text),
                parent_id: Some(parent_sid),
            },
        )?;
        fts::index_code(conn, child_sid, &child_symbol, &c.text, rel)?;
    }
    Ok(())
}

/// Serialise a single extracted symbol as JSONL on stdout. Used by the
/// `--extract` path; no SQLite connection is required.
///
/// JSONL contract (consumed by `comemory ingest-code`): unchunked
/// symbols emit exactly one row. cAST-chunked symbols emit the parent
/// row first (headline snippet, no `parent_symbol` field) followed by
/// one row per chunk carrying two extra fields — `parent_symbol` (the
/// parent's `symbol` value) and `chunk_index` (one-based) — which the
/// ingest side uses to resolve `parent_id` from rows earlier in the
/// same stream.
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

/// Snippet stored on the symbol's own row. Whole symbols keep their full
/// snippet; chunked symbols reduce to a headline of the snippet's first
/// line + the first chunk, so the parent row stays within the line
/// budget while remaining a useful FTS/embedding target. Taking the
/// first line as "the signature" is an approximation — multi-line
/// signatures lose their continuation lines to the chunk rows.
fn parent_snippet_of(s: &ExtractedSymbol) -> Cow<'_, str> {
    match s.chunks.first() {
        None => Cow::Borrowed(&s.snippet),
        Some(first) => Cow::Owned(format!("{}\n{}", first_line_of(&s.snippet), first.text)),
    }
}

/// First line of `snippet` (the signature line for every supported
/// symbol kind); empty snippets yield an empty signature.
fn first_line_of(snippet: &str) -> &str {
    snippet.lines().next().unwrap_or("")
}

/// Symbol name of the `i`-th (zero-based) chunk child: `<name>#<n>`
/// with a one-based `n`, matching the JSONL `chunk_index` field.
fn chunk_symbol(name: &str, i: usize) -> String {
    format!("{}#{}", name, i + 1)
}

/// 64-bit SimHash of `text` ([`simhash::of_body`]), as the i64 the
/// `simhash` column stores. Shared by the parent-row and chunk-row
/// writers on both the DB and JSONL paths.
fn simhash_of(text: &str) -> i64 {
    simhash::of_body(text) as i64
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

/// Render `file` relative to `root` for storage in the `code_symbols.path`
/// column. Falls back to the absolute path when `strip_prefix` fails so we
/// never silently store an empty string.
fn relative(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .unwrap_or(file)
        .to_string_lossy()
        .to_string()
}
