//! `comemory ingest-code` — read pre-embedded code symbol rows from stdin
//! (one JSON object per line) and mirror them into `code_symbols`,
//! `code_fts`, and `code_vec`.
//!
//! Pairs with `comemory index-code --extract`, which emits the same JSONL
//! shape minus the `embedding` field. Callers wedge their own embedder
//! between the two commands when they want vector hits without forcing
//! comemory to download a model.

use std::collections::HashMap;
use std::io::BufRead;
use std::path::PathBuf;

use clap::Args as ClapArgs;
use rusqlite::Connection;
use serde::Deserialize;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::prelude::*;
use crate::store::code_row::{self, CodeSymbolRow};
use crate::store::{connection, fts, vector};

const EXAMPLES: &str = "\
Examples:
  # Pipe pre-embedded JSONL from your embedder into the SQLite store
  comemory index-code --repo myrepo --path . --extract \\
    | embed-snippets \\
    | comemory ingest-code";

/// Arguments to `comemory ingest-code`. Currently no flags — input is the
/// stdin stream of JSONL rows. Wrapped in a struct so future flags can land
/// without breaking the dispatcher signature.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args;

/// JSONL row shape accepted by the ingest path. Mirrors the JSON emitted by
/// `comemory index-code --extract` plus the caller-supplied dense vector.
#[derive(Deserialize)]
struct Row {
    repo: String,
    path: String,
    blob_oid: String,
    symbol: String,
    kind: String,
    lang: String,
    line_start: u32,
    line_end: u32,
    snippet: String,
    simhash: i64,
    embedding: Vec<f32>,
}

/// Drain stdin and insert each row into `code_symbols` + `code_fts` +
/// `code_vec`. The caller-supplied `embedding` is dim-checked by
/// `vector::insert_code` against `schema_meta.code_vector_dim`.
///
/// The whole stdin loop runs inside one SQLite transaction so a malformed
/// row mid-stream rolls back every prior insert — no half-ingested batches
/// land in `code_symbols`/`code_fts`/`code_vec`.
///
/// Each `(repo, path)` tuple is purged via [`code_row::purge_file_symbols`]
/// on its first sighting in the stream so re-ingesting a previously-ingested
/// file (e.g. a fresh embedding pass over the same blob) cannot collide on
/// the `UNIQUE (repo, path, symbol, line_start)` constraint. After the
/// stream is drained, the `indexed_files` cursor is upserted for every
/// `(repo, path)` seen — using the last `blob_oid` observed for that pair —
/// so a follow-up `index-code` run knows the file is already current.
pub async fn run(_args: Args, _json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let mut conn = connection::open(paths.db_path())?;

    let tx = conn.transaction()?;
    // `(repo, path) -> last blob_oid seen` so we know which `indexed_files`
    // rows to refresh once the stream completes.
    let mut seen_files: HashMap<(String, String), String> = HashMap::new();
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = line.map_err(Error::Io)?;
        if line.trim().is_empty() {
            continue;
        }
        let row: Row = serde_json::from_str(&line)?;
        let key = (row.repo.clone(), row.path.clone());
        if !seen_files.contains_key(&key) {
            code_row::purge_file_symbols(&tx, &row.repo, &row.path)?;
        } else if let Some(prev_oid) = seen_files.get(&key) {
            // Reject mixed blob_oid for the same (repo, path) — only the last
            // would otherwise survive in `indexed_files` and a follow-up
            // `index-code` would believe a stale oid is current.
            if prev_oid != &row.blob_oid {
                return Err(Error::Config(format!(
                    "ingest-code: conflicting blob_oid for {}:{} ({} vs {}); \
                     all symbols of one file must share the same blob_oid",
                    row.repo, row.path, prev_oid, row.blob_oid
                )));
            }
        }
        seen_files.insert(key, row.blob_oid.clone());
        insert_row(&tx, &row)?;
    }
    for ((repo, path), oid) in &seen_files {
        code_row::upsert_indexed_file(&tx, repo, path, oid)?;
    }
    tx.commit()?;
    Ok(())
}

/// Insert one parsed JSONL row into the three code tables. Extracted so the
/// stdin loop in `run` stays free of plumbing details.
fn insert_row(conn: &Connection, row: &Row) -> Result<()> {
    let sid = code_row::insert(
        conn,
        &CodeSymbolRow {
            repo: &row.repo,
            path: &row.path,
            blob_oid: &row.blob_oid,
            symbol: &row.symbol,
            kind: &row.kind,
            lang: &row.lang,
            line_start: row.line_start as i64,
            line_end: row.line_end as i64,
            snippet: &row.snippet,
            simhash: row.simhash,
        },
    )?;
    fts::index_code(
        conn,
        sid,
        &row.symbol,
        &row.snippet,
        &fts::path_to_tokens(&row.path),
    )?;
    vector::insert_code(conn, sid, &row.embedding)?;
    Ok(())
}
