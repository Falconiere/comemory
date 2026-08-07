//! `api::ingest_code::run` — shared middle of `comemory ingest-code` /
//! `POST /api/v1/code/ingest`: parse NDJSON rows into `code_symbols` +
//! `code_fts` + `code_vec`. Moved out of `cli::ingest_code::run`.
//!
//! No CLI flags — the real input is the stdin/NDJSON-body stream, so
//! [`run`] takes the body as `&str` rather than a `Request` struct (the
//! per-line [`Row`] already denies unknown fields). `cli::ingest_code::run`
//! reads stdin into a `String` first, then calls this; the HTTP route hands
//! the already-buffered body (bounded by its own 64 MiB limit) straight
//! through.

use std::collections::{BTreeSet, HashMap};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::api::Ctx;
use crate::prelude::*;
use crate::store::code_row::{self, CodeSymbolRow};
use crate::store::{fts, vector};

/// One NDJSON row, mirroring the JSON emitted by `comemory index-code
/// --extract` plus the caller-supplied dense vector. `deny_unknown_fields`
/// catches malformed/extended rows loudly. cAST chunk rows additionally
/// carry `parent_symbol` + `chunk_index` (both or neither): the parent
/// symbol must have appeared earlier in the same stream for the same
/// `(repo, path)`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(default)]
    parent_symbol: Option<String>,
    #[serde(default)]
    chunk_index: Option<u32>,
}

/// `POST /api/v1/code/ingest` response. `cli::ingest_code::run` emits
/// nothing on success; the HTTP job form carries a small real count.
#[derive(Serialize, Debug)]
pub struct Response {
    /// Distinct `(repo, path)` pairs touched by this stream.
    pub files: usize,
    /// Rows inserted.
    pub rows: usize,
}

/// Drain `body`'s NDJSON lines and insert each row into `code_symbols` +
/// `code_fts` + `code_vec`. Runs in one SQLite transaction, so a malformed
/// row mid-stream rolls back every prior insert.
///
/// Each `(repo, path)` tuple is purged on its first sighting so
/// re-ingesting a previously-ingested file cannot collide on the
/// `UNIQUE (repo, path, symbol, line_start)` constraint. After the stream
/// is drained, `indexed_files` is upserted for every `(repo, path)` seen,
/// and every sighted repo's format stamp is refreshed — mirroring
/// `index-code`'s own per-repo format gate so a follow-up `index-code` run
/// does not drop the just-ingested BYO `code_vec` embeddings.
pub fn run(ctx: &mut Ctx<'_>, body: &str) -> Result<Response> {
    let conn = ctx.conn()?;
    let tx = conn.transaction()?;
    let mut state = StreamState::default();
    for line in body.lines() {
        if !line.trim().is_empty() {
            ingest_line(&tx, line, &mut state)?;
        }
    }
    for ((repo, path), oid) in &state.seen_files {
        code_row::upsert_indexed_file(&tx, repo, path, oid)?;
    }
    for repo in &state.seen_repos {
        code_row::stamp_repo_format(&tx, repo)?;
    }
    tx.commit()?;
    Ok(Response {
        files: state.seen_files.len(),
        rows: state.rows,
    })
}

/// Mutable bookkeeping threaded across the whole NDJSON stream by
/// [`ingest_line`]: what's been seen so far, and the running row count.
#[derive(Default)]
struct StreamState {
    /// `(repo, path) -> last blob_oid seen`.
    seen_files: HashMap<(String, String), String>,
    /// Repos sighted so far, for the per-repo format gate + final stamp.
    seen_repos: BTreeSet<String>,
    /// `(repo, path, symbol) -> rowid` for every row inserted so far in
    /// THIS stream, so chunk rows can resolve their parent's rowid.
    inserted_ids: HashMap<(String, String, String), i64>,
    /// Rows inserted so far.
    rows: usize,
}

/// Parse and insert one non-empty NDJSON line, updating `state`.
fn ingest_line(tx: &Connection, line: &str, state: &mut StreamState) -> Result<()> {
    let row: Row = serde_json::from_str(line)?;
    if state.seen_repos.insert(row.repo.clone()) {
        code_row::ensure_repo_format(tx, &row.repo)?;
    }
    let key = (row.repo.clone(), row.path.clone());
    match state.seen_files.get(&key) {
        None => code_row::purge_file_symbols(tx, &row.repo, &row.path)?,
        // Reject mixed blob_oid for the same (repo, path) — only the last
        // would otherwise survive in `indexed_files`.
        Some(prev_oid) if prev_oid != &row.blob_oid => {
            return Err(Error::Config(format!(
                "ingest-code: conflicting blob_oid for {}:{} ({} vs {}); \
                 all symbols of one file must share the same blob_oid",
                row.repo, row.path, prev_oid, row.blob_oid
            )));
        }
        Some(_) => {}
    }
    state.seen_files.insert(key, row.blob_oid.clone());
    let parent_id = resolve_parent(&state.inserted_ids, &row)?;
    let sid = insert_row(tx, &row, parent_id)?;
    state.inserted_ids.insert(
        (row.repo.clone(), row.path.clone(), row.symbol.clone()),
        sid,
    );
    state.rows += 1;
    Ok(())
}

/// Resolve a chunk row's `parent_id` from the rows already inserted in
/// this stream. Plain rows (no `parent_symbol`/`chunk_index`) resolve to
/// `None`; carrying only one of the two fields, or naming a parent that
/// has not appeared earlier in the stream, is a hard error naming the row.
fn resolve_parent(
    inserted_ids: &HashMap<(String, String, String), i64>,
    row: &Row,
) -> Result<Option<i64>> {
    match (&row.parent_symbol, row.chunk_index) {
        (None, None) => Ok(None),
        (Some(parent), Some(_)) => {
            let key = (row.repo.clone(), row.path.clone(), parent.clone());
            match inserted_ids.get(&key) {
                Some(sid) => Ok(Some(*sid)),
                None => Err(Error::Config(format!(
                    "ingest-code: chunk row {}:{}:{} names parent_symbol '{}' \
                     which has not appeared earlier in this stream",
                    row.repo, row.path, row.symbol, parent
                ))),
            }
        }
        _ => Err(Error::Config(format!(
            "ingest-code: row {}:{}:{} must carry both parent_symbol and \
             chunk_index, or neither",
            row.repo, row.path, row.symbol
        ))),
    }
}

/// Insert one parsed row into the three code tables, returning the new
/// `code_symbols` rowid so [`run`] can record it for later chunk rows.
fn insert_row(conn: &Connection, row: &Row, parent_id: Option<i64>) -> Result<i64> {
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
            parent_id,
        },
    )?;
    fts::index_code(conn, sid, &row.symbol, &row.snippet, &row.path)?;
    vector::insert_code(conn, sid, &row.embedding)?;
    Ok(sid)
}
