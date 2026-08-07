//! The `ATTACH`-based preservation copy behind [`super::run`]: everything a
//! markdown replay cannot reconstruct is lifted out of the pre-rebuild DB
//! into the freshly built one — the code index, the mined/earned code-graph
//! edges plus their per-repo cursors, and the learning-loop counters.
//!
//! Every copy lists its columns explicitly and probes the attached DB for
//! columns added by later migrations: the old DB is attached raw and never
//! migrated, so a `SELECT *` would break the moment the source predates a
//! widening migration.

use std::path::Path;

use crate::graph::edges::CO_ACTIVATED;
use crate::prelude::*;
use crate::store::code_row;

/// Attach `old_db` as `old` and copy the code-index tables (+ mined
/// code-graph edges and `repo_marker` cursors) plus the five learning
/// tables into the already-open `conn` (which points at the tmp path).
/// Uses INSERT-SELECT so no intermediate buffers are needed; runs outside
/// a transaction because vec0 virtual tables cannot participate in user
/// transactions.
///
/// The ATTACH path is bound via a parameter rather than interpolated into the
/// SQL so a data dir whose path contains a single quote (or other SQL
/// metacharacter) cannot break the statement.
///
/// Each source table is only copied if it actually exists on the attached
/// database: a v0.1 or otherwise legacy `comemory.db` may not have any of the
/// v2 code-index tables (and a pre-v5 one lacks `feedback_events` /
/// `query_expansions`), in which case the rebuild should still succeed
/// rather than failing with "no such table".
pub(crate) fn copy_preserved_tables_from_old(
    conn: &mut rusqlite::Connection,
    old_db: &Path,
) -> Result<()> {
    conn.execute(
        "ATTACH DATABASE ? AS old",
        rusqlite::params![old_db.to_string_lossy().as_ref()],
    )?;
    let copy_result = copy_code_tables_inner(conn).and_then(|()| copy_learning_tables_inner(conn));
    // Always DETACH so the connection is reusable even if the copy failed.
    let _ = conn.execute_batch("DETACH DATABASE old;");
    copy_result
}

/// Inner copy loop separated so [`copy_preserved_tables_from_old`] can
/// guarantee `DETACH` runs even on error.
///
/// A pre-v4 `code_symbols` lacks the `access_count` / `last_accessed`
/// columns added by migration 0004, so those two are sourced conditionally:
/// carried over when the old table already has them, otherwise synthesized
/// with the same defaults 0004's backfill applies (`0` / `indexed_at`). The
/// v6 columns (`rank_score` / `parent_id`, added together by 0006) are
/// probed the same way and synthesized with the 0006 defaults (`0.0` /
/// NULL). `id` is carried verbatim so `parent_id` chunk → parent pointers
/// stay valid in the copy.
fn copy_code_tables_inner(conn: &rusqlite::Connection) -> Result<()> {
    // Regular tables first, then the virtual ones (FTS5 + vec0):
    // `code_symbols` must land before `code_vec` / `code_fts` because the
    // latter reference `code_symbols.id` in their data streams.
    copy_code_index_tables(conn)?;
    copy_mined_edges(conn)?;
    copy_code_markers(conn)?;
    copy_code_virtual_tables(conn)
}

/// Copy the `code_symbols` rows and the `indexed_files` cursors.
fn copy_code_index_tables(conn: &rusqlite::Connection) -> Result<()> {
    if old_table_exists(conn, "code_symbols")? {
        let (count_expr, last_expr) = if old_column_exists(conn, "code_symbols", "access_count")? {
            ("access_count", "COALESCE(last_accessed, indexed_at)")
        } else {
            ("0", "indexed_at")
        };
        let (rank_expr, parent_expr) = if old_column_exists(conn, "code_symbols", "rank_score")? {
            ("rank_score", "parent_id")
        } else {
            ("0.0", "NULL")
        };
        conn.execute_batch(&format!(
            "INSERT OR IGNORE INTO main.code_symbols(\
                 id, repo, path, blob_oid, symbol, kind, lang, line_start, line_end, \
                 snippet, simhash, indexed_at, access_count, last_accessed, \
                 rank_score, parent_id) \
             SELECT id, repo, path, blob_oid, symbol, kind, lang, line_start, line_end, \
                 snippet, simhash, indexed_at, {count_expr}, {last_expr}, \
                 {rank_expr}, {parent_expr} \
             FROM old.code_symbols;"
        ))?;
    }
    if old_table_exists(conn, "indexed_files")? {
        conn.execute_batch(
            "INSERT OR IGNORE INTO main.indexed_files(repo, path, blob_oid, indexed_at) \
             SELECT repo, path, blob_oid, indexed_at FROM old.indexed_files;",
        )?;
    }
    Ok(())
}

/// Copy the mined/earned code-graph edges. The rel filter narrows to the
/// three kinds the markdown replay cannot reproduce: the git-mined
/// `co_changed` / `imports` edges plus the v8 `co_activated` edges earned by
/// the co-activation reward (memory→file, weighted — earned state markdown
/// has no source for, like the feedback counters). A pre-v6 source has no
/// such rows (its rel CHECK predates the kinds) and no `weight` column,
/// hence the probe defaulting to the pre-v6 implicit weight of 1. The
/// [`CO_ACTIVATED`] const is bound rather than inlined so the filter cannot
/// drift from the writer's literal; it is a crate-internal const with no SQL
/// metacharacters, so interpolation is safe.
fn copy_mined_edges(conn: &rusqlite::Connection) -> Result<()> {
    if old_table_exists(conn, "edges")? {
        let weight_expr = if old_column_exists(conn, "edges", "weight")? {
            "weight"
        } else {
            "1"
        };
        conn.execute_batch(&format!(
            "INSERT OR IGNORE INTO main.edges(\
                 src_kind, src_id, dst_kind, dst_id, rel, weight, created_at) \
             SELECT src_kind, src_id, dst_kind, dst_id, rel, {weight_expr}, created_at \
             FROM old.edges WHERE rel IN ('co_changed', 'imports', '{CO_ACTIVATED}');"
        ))?;
    }
    Ok(())
}

/// Copy the per-repo cursors an `index-code` pass reads before deciding what
/// to re-walk: the `code_format:<repo>` stamps in `schema_meta` (matched on
/// [`code_row::CODE_FORMAT_KEY_PREFIX`] — the global `code_format_version`
/// key lacks the colon and does NOT match; without them the next index-code
/// sees an unstamped repo, drops its `indexed_files` cursors, and the full
/// re-walk purges the BYO `code_vec` rows), plus the `repo_marker` rows
/// whose `last_mined_commit` bounds the next mining pass (dropping it would
/// re-mine bounded history into the just-copied co_changed weights,
/// double-counting every pair). Both prefixes are crate-internal consts with
/// no SQL metacharacters, so the interpolation cannot break the statements.
fn copy_code_markers(conn: &rusqlite::Connection) -> Result<()> {
    if old_table_exists(conn, "schema_meta")? {
        let prefix = code_row::CODE_FORMAT_KEY_PREFIX;
        conn.execute_batch(&format!(
            "INSERT OR IGNORE INTO main.schema_meta(key, value) \
             SELECT key, value FROM old.schema_meta \
              WHERE substr(key, 1, {len}) = '{prefix}';",
            len = prefix.len(),
        ))?;
    }
    if old_table_exists(conn, "repo_marker")? {
        let mined_expr = if old_column_exists(conn, "repo_marker", "last_mined_commit")? {
            "last_mined_commit"
        } else {
            "NULL"
        };
        conn.execute_batch(&format!(
            "INSERT OR IGNORE INTO main.repo_marker(\
                 repo, last_head, last_indexed_at, last_mined_commit) \
             SELECT repo, last_head, last_indexed_at, {mined_expr} \
             FROM old.repo_marker;"
        ))?;
    }
    Ok(())
}

/// Copy the FTS5 + vec0 virtual tables. These may not support
/// `INSERT INTO … SELECT *` from an attached DB in all sqlite-vec versions,
/// so each row is copied via named columns: `code_fts` through the FTS5
/// content-table shape, `code_vec` as blobs tied to `symbol_id`.
fn copy_code_virtual_tables(conn: &rusqlite::Connection) -> Result<()> {
    if old_table_exists(conn, "code_fts")? {
        conn.execute_batch(
            "INSERT OR IGNORE INTO main.code_fts(symbol_id, symbol, snippet, path_tokens) \
             SELECT symbol_id, symbol, snippet, path_tokens FROM old.code_fts;",
        )?;
    }
    if old_table_exists(conn, "code_vec")? {
        conn.execute_batch(
            "INSERT OR IGNORE INTO main.code_vec(symbol_id, embedding) \
             SELECT symbol_id, embedding FROM old.code_vec;",
        )?;
    }
    Ok(())
}

/// Inner copy loop for the learning-loop tables. These rows exist only in
/// SQLite — there is no markdown to rebuild them from — so dropping them
/// would silently reset the Beta feedback rerank priors to neutral and erase
/// mined expansions, contradicting the documented never-expire contract.
///
/// Same schema-evolution guards as [`copy_code_tables_inner`]: each table is
/// copied only when it exists on the attached DB, and columns added by later
/// migrations are probed and defaulted per callee.
fn copy_learning_tables_inner(conn: &rusqlite::Connection) -> Result<()> {
    copy_feedback_tables(conn)?;
    copy_retrieval_log(conn)?;
    copy_event_and_mined_tables(conn)
}

/// Copy the aggregated feedback counters: memory-side `feedback` (v2) and
/// symbol-side `code_feedback` (v6). The `repo` column probe also covers the
/// brief dev-era rowid-keyed `code_feedback` shape (never released): skip
/// rather than abort.
fn copy_feedback_tables(conn: &rusqlite::Connection) -> Result<()> {
    if old_table_exists(conn, "feedback")? {
        conn.execute_batch(
            "INSERT OR IGNORE INTO main.feedback(\
                 memory_id, used_count, irrelevant_count, last_used) \
             SELECT memory_id, used_count, irrelevant_count, last_used \
             FROM old.feedback;",
        )?;
    }
    if old_table_exists(conn, "code_feedback")? && old_column_exists(conn, "code_feedback", "repo")?
    {
        conn.execute_batch(
            "INSERT OR IGNORE INTO main.code_feedback(\
                 repo, path, symbol, used_count, irrelevant_count, last_used) \
             SELECT repo, path, symbol, used_count, irrelevant_count, last_used \
             FROM old.code_feedback;",
        )?;
    }
    Ok(())
}

/// Copy the `retrieval_log` telemetry (v3). `duration_ms` (v5) and the
/// `repo` / `kind` / `source` filter columns (v6, probed together via
/// `source`) default to NULL / NULL / NULL / `'search'` when the source
/// predates them — without that, old `search-code` rows would re-enter
/// reformulation mining as memory queries.
fn copy_retrieval_log(conn: &rusqlite::Connection) -> Result<()> {
    if old_table_exists(conn, "retrieval_log")? {
        let duration_expr = if old_column_exists(conn, "retrieval_log", "duration_ms")? {
            "duration_ms"
        } else {
            "NULL"
        };
        let (repo_expr, kind_expr, source_expr) =
            if old_column_exists(conn, "retrieval_log", "source")? {
                ("repo", "kind", "source")
            } else {
                ("NULL", "NULL", "'search'")
            };
        conn.execute_batch(&format!(
            "INSERT OR IGNORE INTO main.retrieval_log(\
                 query_id, query, returned_ids, at, duration_ms, repo, kind, source) \
             SELECT query_id, query, returned_ids, at, {duration_expr}, \
                 {repo_expr}, {kind_expr}, {source_expr} \
             FROM old.retrieval_log;"
        ))?;
    }
    Ok(())
}

/// Copy the `feedback_events` verdict log (v5), the mined
/// `query_expansions` (v5), and the `bandit_arms` state. On pre-migration
/// sources `target_kind` (v6) defaults to `'memory'` and `provenance` (v8)
/// to `'manual'`, the same values those migrations backfill — dropping
/// either would let code verdicts masquerade as memory verdicts in the
/// harvest, or relabel implicit reinforcement as a user verdict.
fn copy_event_and_mined_tables(conn: &rusqlite::Connection) -> Result<()> {
    if old_table_exists(conn, "feedback_events")? {
        let target_expr = if old_column_exists(conn, "feedback_events", "target_kind")? {
            "target_kind"
        } else {
            "'memory'"
        };
        let prov_expr = if old_column_exists(conn, "feedback_events", "provenance")? {
            "provenance"
        } else {
            "'manual'"
        };
        conn.execute_batch(&format!(
            "INSERT OR IGNORE INTO main.feedback_events(\
                 id, query_id, memory_id, verdict, at, target_kind, provenance) \
             SELECT id, query_id, memory_id, verdict, at, {target_expr}, {prov_expr} \
             FROM old.feedback_events;"
        ))?;
    }
    if old_table_exists(conn, "query_expansions")? {
        conn.execute_batch(
            "INSERT OR IGNORE INTO main.query_expansions(\
                 term, expansion, support, last_mined) \
             SELECT term, expansion, support, last_mined \
             FROM old.query_expansions;",
        )?;
    }
    if old_table_exists(conn, "bandit_arms")? {
        conn.execute_batch(
            "INSERT OR IGNORE INTO main.bandit_arms(\
                 arm_id, rrf_k, decay, mmr_lambda, bm25_body, bm25_tags, \
                 alpha, beta, pulls, last_mrr, updated_at) \
             SELECT arm_id, rrf_k, decay, mmr_lambda, bm25_body, bm25_tags, \
                 alpha, beta, pulls, last_mrr, updated_at \
             FROM old.bandit_arms;",
        )?;
    }
    Ok(())
}

/// True when `name` exists as a table (regular or virtual) on the attached
/// `old` database. Lets the copy loop skip tables that predate v0.2.
fn old_table_exists(conn: &rusqlite::Connection, name: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT count(*) FROM old.sqlite_master WHERE type = 'table' AND name = ?1",
        rusqlite::params![name],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

/// True when `column` exists on `table` in the attached `old` database.
/// Lets [`copy_code_tables_inner`] and [`copy_learning_tables_inner`] adapt
/// their SELECT lists to the attached DB's schema version instead of
/// assuming the current one.
fn old_column_exists(conn: &rusqlite::Connection, table: &str, column: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT count(*) FROM pragma_table_info(?1, 'old') WHERE name = ?2",
        rusqlite::params![table, column],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}
