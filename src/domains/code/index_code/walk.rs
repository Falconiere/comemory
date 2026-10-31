//! Walk-path internals for `domains::code::index_code`: per-file symbol extraction
//! and blob-OID bookkeeping. `blob_oid`, `relative`, `parent_snippet_of`,
//! `chunk_symbol`, and `simhash_of` are also reused by `cli::index_code`'s
//! CLI-only `--extract` path, which emits the same JSONL shape without a
//! SQLite connection.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use git2::Repository;

use crate::domains::code::ast::extractor::ExtractedSymbol;
use crate::domains::code::ast::{self, languages};
use crate::domains::graph::imports;
use crate::prelude::*;
use crate::store::code_row::{self, CodeSymbolRow};
use crate::store::{Connection, code_embedding, fts, indexed_files};
use crate::utilities::progress::ProgressSink;
use crate::utilities::simhash;

/// Index one walked file into the caller's open transaction, returning
/// whether it was actually (re)indexed. `None` means unsupported or
/// untracked; `Some(false)` means the working bytes are already indexed.
/// Errors abort the whole walk, so the caller's transaction rolls back.
/// `sink`, when given, gets one [`ProgressSink::on_log`] line (the file's
/// relative path) for each file actually (re)indexed — not for a skip.
pub(crate) fn index_file(
    tx: &Connection,
    repo: &str,
    root: &Path,
    source: (Option<&Repository>, u64),
    path: &Path,
    imports_by_file: &mut BTreeMap<String, Vec<String>>,
    sink: Option<&dyn ProgressSink>,
) -> Result<Option<bool>> {
    let Some(lang) = languages::detect(path) else {
        return Ok(None);
    };
    let rel = relative(root, path);
    if let Some(git) = source.0
        && !tracked_file(git, path)?
    {
        return Ok(None);
    }
    let Some(snippet) = read_source(path, source.1)? else {
        return Ok(None);
    };
    let oid = git2::Oid::hash_object(git2::ObjectType::Blob, snippet.as_bytes())?.to_string();
    if oid_is_indexed(tx, repo, &rel, &oid)? {
        return Ok(Some(false));
    }
    // Drop any prior `code_symbols`/`code_vec`/`code_fts` rows for this
    // `(repo, path)` before re-inserting so a blob_oid change (file edited)
    // does not leave stale symbol rows behind, and so re-inserts with
    // shifted `line_start` values cannot collide on the
    // `UNIQUE(repo, path, symbol, line_start)` constraint mid-transaction.
    let saved = code_embedding::capture(tx, repo, &rel)?;
    code_row::purge_file_symbols(tx, repo, &rel)?;
    for s in &ast::extract(lang, &snippet)? {
        write_symbol(tx, repo, &rel, &oid, lang, s)?;
    }
    code_embedding::restore(tx, repo, &rel, &saved)?;
    // Collect this file's raw imports for the graph post-pass. An empty
    // Vec still matters — it clears the file's stale `imports` edges.
    match imports::extract_imports(lang, &snippet) {
        Ok(modules) => {
            imports_by_file.insert(rel.clone(), modules);
        }
        Err(e) => {
            tracing::warn!(file = %rel, error = %e, "index-code: import extraction failed");
        }
    }
    code_row::upsert_indexed_file(tx, repo, &rel, &oid)?;
    if let Some(sink) = sink {
        sink.on_log(&rel);
    }
    Ok(Some(true))
}

fn read_source(path: &Path, max: u64) -> Result<Option<String>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(max.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max || bytes.contains(&0) {
        return Ok(None);
    }
    Ok(String::from_utf8(bytes).ok())
}

/// Check Git tracking without treating an unreadable index as an absence.
pub(super) fn tracked_file(repo: &Repository, path: &Path) -> Result<bool> {
    let workdir = repo
        .workdir()
        .ok_or_else(|| Error::Other("Git worktree is unavailable".into()))?
        .canonicalize()?;
    let file = path.canonicalize()?;
    let rel = file
        .strip_prefix(workdir)
        .map_err(|_| Error::Forbidden("code file leaves Git worktree".into()))?;
    Ok(repo.index()?.get_path(rel, 0).is_some())
}

/// Persist the absolute working-tree root so `comemory serve` can resolve a
/// `file:<repo>:<path>` node id back to a real file. A canonicalize
/// failure (path vanished mid-run) leaves `root_path` NULL.
pub(crate) fn stamp_repo_root(tx: &Connection, repo: &str, root: &Path) -> Result<()> {
    match root.canonicalize() {
        Ok(canon) => code_row::upsert_repo_root(tx, repo, &canon.to_string_lossy()),
        Err(e) => {
            tracing::warn!(
                path = %root.display(),
                error = %e,
                "index-code: could not canonicalize path for repo root; serve will need --root",
            );
            Ok(())
        }
    }
}

/// Insert a single extracted symbol into `code_symbols` + `code_fts`.
/// Unchunked symbols land as one row carrying the full snippet. cAST-
/// chunked symbols land as one PARENT row plus one CHILD row per chunk
/// (symbol `<name>#<n>`, `parent_id` pointing at the parent's rowid).
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

/// Snippet stored on the symbol's own row. Whole symbols keep their full
/// snippet; chunked symbols reduce to a headline of the snippet's first
/// line + the first chunk.
pub(crate) fn parent_snippet_of(s: &ExtractedSymbol) -> Cow<'_, str> {
    match s.chunks.first() {
        None => Cow::Borrowed(&s.snippet),
        Some(first) => Cow::Owned(format!("{}\n{}", first_line_of(&s.snippet), first.text)),
    }
}

/// First line of `snippet`; empty snippets yield an empty signature.
fn first_line_of(snippet: &str) -> &str {
    snippet.lines().next().unwrap_or("")
}

/// Symbol name of the `i`-th (zero-based) chunk child: `<name>#<n>` with a
/// one-based `n`.
pub(crate) fn chunk_symbol(name: &str, i: usize) -> String {
    format!("{}#{}", name, i + 1)
}

/// 64-bit SimHash of `text`, as the i64 the `simhash` column stores.
pub(crate) fn simhash_of(text: &str) -> i64 {
    simhash::of_body(text) as i64
}

/// Returns true when `indexed_files` already records `oid` for
/// `repo + path` — the working-tree blob hasn't changed since the last run.
fn oid_is_indexed(conn: &Connection, repo: &str, path: &str, oid: &str) -> Result<bool> {
    Ok(indexed_files::blob_oid_for(conn, repo, path)?.as_deref() == Some(oid))
}

/// Look up the working-tree blob OID for `file` via the repo index. Files
/// that are untracked (or unreadable through git) return `None`.
///
/// `libgit2`'s `Index::get_path` rejects absolute paths, so both the
/// working tree and the file path are canonicalized before stripping the
/// prefix. Canonicalisation failures are logged via `tracing::warn!`.
pub(crate) fn blob_oid(repo: &Repository, file: &Path) -> Option<String> {
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

/// Render `file` relative to `root` for storage in `code_symbols.path`.
/// Falls back to the absolute path when `strip_prefix` fails.
pub(crate) fn relative(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .unwrap_or(file)
        .to_string_lossy()
        .to_string()
}

#[cfg(test)]
#[path = "tests/walk.rs"]
mod tests;
