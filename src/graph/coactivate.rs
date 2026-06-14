//! Commit co-activation reward: commits touching files a memory references
//! reinforce that memory.
//!
//! Runs inside `materialize`'s transaction (after PageRank, before the mining
//! cursor advances) so the reward is atomic with the cursor — a crash can't
//! half-apply it and the cursor harvests each commit's touch count exactly
//! once. Three reinforcement channels per affected memory: a weighted
//! `co_activated` edge, a one-shot Beta `used` minted when the edge weight
//! first reaches `>= 2`, and one `access_count` bump + `last_accessed = at`.
//!
//! Determinism/idempotency: the edge upsert is `old + delta`, the Beta
//! crossing is a pure function of `(old_weight, delta)`, and re-running over
//! the same commits is a no-op (the cursor makes every `delta` zero).

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;

use crate::graph::edges::{self, EdgeKey, REFERENCES_FILE, file_node_id};
use crate::prelude::*;
use crate::stats::feedback;

/// Max bound variables per `IN (...)` chunk — well under bundled SQLite's
/// `SQLITE_MAX_VARIABLE_NUMBER` (32766 in 3.46), so a large touch set never
/// trips the host-parameter cap.
const IN_CHUNK: usize = 500;

/// Edge weight at which a `co_activated` edge mints its single Beta `used`.
const BETA_THRESHOLD: i64 = 2;

/// Apply the commit co-activation reward for `repo`: for every `(memory,
/// file)` pair where the memory `references_file` a file touched this pass,
/// accumulate the `co_activated` edge weight by the file's touch count, mint
/// a one-shot Beta `used` when the weight first crosses [`BETA_THRESHOLD`],
/// and bump each reinforced memory's activation once.
///
/// `touched` maps repo-relative paths to per-pass commit-touch counts (from
/// [`crate::graph::cochange::MineOutcome::touched`]). `at` is the run
/// timestamp for `feedback_events.at` and `memories.last_accessed` —
/// commit-time crediting is a deliberate deferral. A `conn` that is a
/// `rusqlite::Transaction` keeps the whole reward atomic with the caller.
pub(crate) fn harvest(
    conn: &Connection,
    repo: &str,
    touched: &HashMap<String, u32>,
    at: &str,
) -> Result<()> {
    if touched.is_empty() {
        return Ok(());
    }
    let pairs = referencing_memories(conn, repo, touched)?;
    let mut reinforced: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for Pair { memory_id, path } in &pairs {
        let Some(&delta) = touched.get(path.as_str()) else {
            continue;
        };
        if delta == 0 {
            continue;
        }
        reward_pair(conn, repo, memory_id, path, i64::from(delta), at)?;
        if seen.insert(memory_id.clone()) {
            reinforced.push(memory_id.clone());
        }
    }
    bump_activation(conn, &reinforced, at)?;
    Ok(())
}

/// One `(memory_id, referenced repo-relative path)` row from the reverse
/// `references_file` query.
struct Pair {
    memory_id: String,
    path: String,
}

/// Reverse-resolve every memory whose `references_file` edge points at a
/// touched file. The cross-link writer stores those dst_ids in the BARE
/// `<repo>:<path>` form (no `file:` prefix — see
/// [`crate::graph::edges::file_node_id`]), so the candidate dst_ids match.
/// The `idx_edges_dst` index backs the lookup; the IN-list is chunked under
/// [`IN_CHUNK`] so a large touch set cannot exceed the host-parameter cap.
fn referencing_memories(
    conn: &Connection,
    repo: &str,
    touched: &HashMap<String, u32>,
) -> Result<Vec<Pair>> {
    // Sorted candidates → deterministic chunk boundaries and stable order.
    let mut dst_ids: Vec<String> = touched.keys().map(|p| format!("{repo}:{p}")).collect();
    dst_ids.sort();
    let prefix = format!("{repo}:");
    let mut out: Vec<Pair> = Vec::new();
    for chunk in dst_ids.chunks(IN_CHUNK) {
        let qmarks = crate::store::qmarks(chunk.len());
        let sql = format!(
            "SELECT src_id, dst_id FROM edges \
              WHERE rel = ?1 AND dst_kind = 'file' AND dst_id IN ({qmarks}) \
              ORDER BY dst_id, src_id"
        );
        let mut stmt = conn.prepare(&sql)?;
        let params = std::iter::once(REFERENCES_FILE).chain(chunk.iter().map(String::as_str));
        let rows = stmt
            .query_map(rusqlite::params_from_iter(params), |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for (memory_id, dst_id) in rows {
            // Strip the `<repo>:` head to recover the touch-map path key.
            let Some(path) = dst_id.strip_prefix(&prefix) else {
                continue;
            };
            out.push(Pair {
                memory_id,
                path: path.to_string(),
            });
        }
    }
    Ok(out)
}

/// Accumulate the `(memory→file, co_activated)` edge weight by `delta` and,
/// when the weight first crosses [`BETA_THRESHOLD`], mint one implicit `used`
/// stamped with the run timestamp `at`. The co_activated edge uses the
/// canonical `file:` node id (per `0008_v8_reinforcement.sql`); the reverse
/// lookup keyed off the bare cross-link form — each rel keeps its own id
/// grammar.
///
/// The crossing reads the stored weight BEFORE the upsert, so it is a pure
/// function of `(old_weight, delta)`: idempotent across re-runs because the
/// mining cursor makes `delta` zero for already-counted commits.
fn reward_pair(
    conn: &Connection,
    repo: &str,
    memory_id: &str,
    path: &str,
    delta: i64,
    at: &str,
) -> Result<()> {
    let dst = file_node_id(repo, path);
    let key = EdgeKey {
        src_kind: "memory",
        src_id: memory_id,
        dst_kind: "file",
        dst_id: &dst,
        rel: edges::CO_ACTIVATED,
    };
    let old = edges::current_weight(conn, key)?;
    edges::insert_weighted(conn, key, delta)?;
    if old < BETA_THRESHOLD && old + delta >= BETA_THRESHOLD {
        feedback::record_implicit_used(conn, memory_id, at)?;
    }
    Ok(())
}

/// Bump `access_count` + `last_accessed = at` once per reinforced memory in a
/// single chunked `UPDATE ... WHERE id IN (...)`. Empty input is a no-op.
fn bump_activation(conn: &Connection, ids: &[String], at: &str) -> Result<()> {
    for chunk in ids.chunks(IN_CHUNK) {
        if chunk.is_empty() {
            continue;
        }
        let qmarks = crate::store::qmarks(chunk.len());
        let sql = format!(
            "UPDATE memories SET access_count = access_count + 1, last_accessed = ?1 \
              WHERE id IN ({qmarks})"
        );
        let params = std::iter::once(at).chain(chunk.iter().map(String::as_str));
        conn.execute(&sql, rusqlite::params_from_iter(params))?;
    }
    Ok(())
}
