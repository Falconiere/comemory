//! Commit co-activation reward: commits touching files a memory references
//! reinforce that memory.
//!
//! Runs inside `materialize`'s transaction (after PageRank, before the mining
//! cursor advances) so the reward is atomic with the cursor — a crash can't
//! half-apply it and the cursor harvests each commit's touch count exactly
//! once. Three reinforcement channels per affected memory: a weighted
//! `co_activated` edge, a one-shot Beta `used` minted when the edge weight
//! first reaches `>= 2` (provenance `auto_coactivation` or `auto_search_edit`
//! when the memory also appeared on a recent search/context page), and one
//! `access_count` bump + `last_accessed = at`.
//!
//! Determinism/idempotency: the edge upsert is `old + delta`, the Beta
//! crossing is a pure function of `(old_weight, delta)`, and re-running over
//! the same commits is a no-op (the cursor makes every `delta` zero).

use std::collections::{HashMap, HashSet};

use crate::graph::search_edit;
use crate::prelude::*;
use crate::stats::feedback;
use crate::store::Connection;
use crate::store::edges::{self, EdgeKey, REFERENCES_FILE, file_node_id};
use crate::store::memory_row;

/// Max bound variables per `IN (...)` chunk — well under bundled SQLite's
/// `SQLITE_MAX_VARIABLE_NUMBER` (32766 in 3.46), so a large touch set never
/// trips the host-parameter cap.
const IN_CHUNK: usize = 500;

/// Edge weight at which a `co_activated` edge mints its single Beta `used`.
const BETA_THRESHOLD: i64 = 2;

/// Apply the commit co-activation reward for `repo`: for every `(memory,
/// file)` pair where the memory `references_file` a file touched this pass,
/// accumulate the `co_activated` edge weight by the file's touch count, mint
/// a one-shot Beta `used` when the weight first crosses [`BETA_THRESHOLD`]
/// (search→edit provenance when the memory was recently returned by search
/// or context), and bump each reinforced memory's activation once.
///
/// `touched` maps repo-relative paths to per-pass commit-touch counts (from
/// [`crate::graph::cochange::MineOutcome::touched`]). `at` is the run
/// timestamp for `feedback_events.at` and `memories.last_accessed`.
/// `lookback_days` bounds the search→edit `retrieval_log` window.
pub(crate) fn harvest(
    conn: &Connection,
    repo: &str,
    touched: &HashMap<String, u32>,
    at: &str,
    lookback_days: u32,
) -> Result<()> {
    if touched.is_empty() {
        return Ok(());
    }
    let pairs = referencing_memories(conn, repo, touched)?;
    let candidates: HashSet<String> = pairs.iter().map(|p| p.memory_id.clone()).collect();
    let search_edit_hits =
        search_edit::memories_seen_recently(conn, repo, &candidates, at, lookback_days)?;
    let mut reinforced: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for Pair { memory_id, path } in &pairs {
        let Some(&delta) = touched.get(path.as_str()) else {
            continue;
        };
        if delta == 0 {
            continue;
        }
        reward_pair(
            conn,
            repo,
            memory_id,
            path,
            i64::from(delta),
            at,
            search_edit_hits.contains(memory_id),
        )?;
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
/// [`crate::store::edges::file_node_id`]), so the candidate dst_ids match.
fn referencing_memories(
    conn: &Connection,
    repo: &str,
    touched: &HashMap<String, u32>,
) -> Result<Vec<Pair>> {
    let mut dst_ids: Vec<String> = touched.keys().map(|p| format!("{repo}:{p}")).collect();
    dst_ids.sort();
    let prefix = format!("{repo}:");
    let mut out: Vec<Pair> = Vec::new();
    for chunk in dst_ids.chunks(IN_CHUNK) {
        let chunk_refs: Vec<&str> = chunk.iter().map(String::as_str).collect();
        let rows = edges::src_ids_for_dst_ids(conn, REFERENCES_FILE, &chunk_refs)?;
        for (memory_id, dst_id) in rows {
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
/// with search→edit or co-activation provenance.
fn reward_pair(
    conn: &Connection,
    repo: &str,
    memory_id: &str,
    path: &str,
    delta: i64,
    at: &str,
    search_edit: bool,
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
        let (prov, qid) = if search_edit {
            (
                feedback::PROV_AUTO_SEARCH_EDIT,
                feedback::SEARCH_EDIT_QUERY_ID,
            )
        } else {
            (
                feedback::PROV_AUTO_COACTIVATION,
                feedback::COACTIVATION_QUERY_ID,
            )
        };
        feedback::record_implicit_used(conn, memory_id, at, prov, qid)?;
    }
    Ok(())
}

/// Bump `access_count` + `last_accessed = at` once per reinforced memory in a
/// single chunked `UPDATE ... WHERE id IN (...)`. Empty input is a no-op.
fn bump_activation(conn: &Connection, ids: &[String], at: &str) -> Result<()> {
    for chunk in ids.chunks(IN_CHUNK) {
        memory_row::bump_access(conn, chunk, at)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/coactivate.rs"]
mod tests;
