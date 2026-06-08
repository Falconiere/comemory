//! `comemory ingest-code` — read pre-embedded code symbol rows from stdin
//! (one JSON object per line) and mirror them into `code_symbols`,
//! `code_fts`, and `code_vec`.
//!
//! Pairs with `comemory index-code --extract`, which emits the same JSONL
//! shape minus the `embedding` field. Callers wedge their own embedder
//! between the two commands when they want vector hits without forcing
//! comemory to download a model.

use std::io::BufRead;
use std::path::PathBuf;

use clap::Args as ClapArgs;
use rusqlite::Connection;
use serde::Deserialize;

use crate::cli::resolve_data_dir;
use crate::config::paths::Paths;
use crate::prelude::*;
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
pub async fn run(_args: Args, _json: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let mut conn = connection::open(paths.db_path())?;

    let tx = conn.transaction()?;
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = line.map_err(Error::Io)?;
        if line.trim().is_empty() {
            continue;
        }
        let row: Row = serde_json::from_str(&line)?;
        insert_row(&tx, &row)?;
    }
    tx.commit()?;
    Ok(())
}

/// Insert one parsed JSONL row into the three code tables. Extracted so the
/// stdin loop in `run` stays free of plumbing details.
fn insert_row(conn: &Connection, row: &Row) -> Result<()> {
    let sid: i64 = conn.query_row(
        "INSERT INTO code_symbols(\
             repo, path, blob_oid, symbol, kind, lang, \
             line_start, line_end, snippet, simhash, indexed_at) \
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10, strftime('%Y-%m-%dT%H:%M:%fZ','now')) \
         RETURNING id",
        rusqlite::params![
            &row.repo,
            &row.path,
            &row.blob_oid,
            &row.symbol,
            &row.kind,
            &row.lang,
            row.line_start as i64,
            row.line_end as i64,
            &row.snippet,
            row.simhash,
        ],
        |r| r.get(0),
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
