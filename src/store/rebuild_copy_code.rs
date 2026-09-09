//! The code-index half of [`super::rebuild_copy`]'s preservation copy:
//! `code_symbols`, `indexed_files`, the mined `co_changed`/`imports`/
//! `co_activated` edges, the per-repo `schema_meta`/`repo_marker` cursors,
//! and the `code_fts`/`code_vec` virtual tables.
//!
//! A pre-v4 `code_symbols` lacks `access_count`/`last_accessed` (added by
//! migration 0004), so those two are sourced conditionally: carried over
//! when present, otherwise synthesized with 0004's own backfill defaults
//! (`0` / `indexed_at`). The v6 columns (`rank_score`/`parent_id`, added
//! together by 0006) are probed the same way, defaulting to `0.0` / NULL.

use crate::prelude::*;
use crate::store::edges::CO_ACTIVATED;
use crate::store::rebuild_copy::{old_column_exists, old_table_exists};
use crate::store::{Connection, code_row};

/// Regular tables first, then the virtual ones (FTS5 + vec0): `code_symbols`
/// must land before `code_vec`/`code_fts` because the latter reference
/// `code_symbols.id` in their data streams.
pub(crate) fn copy_code_tables_inner(conn: &Connection) -> Result<()> {
    copy_code_index_tables(conn)?;
    copy_mined_edges(conn)?;
    copy_code_markers(conn)?;
    copy_code_virtual_tables(conn)
}

/// Copy the `code_symbols` rows and the `indexed_files` cursors.
fn copy_code_index_tables(conn: &Connection) -> Result<()> {
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
/// three kinds markdown replay cannot reproduce: git-mined `co_changed` /
/// `imports` plus the v8 `co_activated` edges earned by the co-activation
/// reward. A pre-v6 source has no such rows and no `weight` column, hence
/// the probe defaulting to the pre-v6 implicit weight of 1. [`CO_ACTIVATED`]
/// is bound rather than inlined so the filter cannot drift from the
/// writer's literal.
fn copy_mined_edges(conn: &Connection) -> Result<()> {
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
/// [`code_row::CODE_FORMAT_KEY_PREFIX`]; without them the next index-code
/// drops its `indexed_files` cursors and purges the BYO `code_vec` rows),
/// plus the `repo_marker` rows whose `last_mined_commit` bounds the next
/// mining pass (dropping it would re-mine bounded history into the
/// just-copied `co_changed` weights, double-counting every pair). Both
/// prefixes are crate-internal consts with no SQL metacharacters.
fn copy_code_markers(conn: &Connection) -> Result<()> {
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
        // `root_path` (v7) and `archived` (v15) are probed the same way:
        // dropping either would make a rebuilt store forget where a repo
        // lives (containment, freshness) or that it was archived.
        let root_expr = if old_column_exists(conn, "repo_marker", "root_path")? {
            "root_path"
        } else {
            "NULL"
        };
        let archived_expr = if old_column_exists(conn, "repo_marker", "archived")? {
            "archived"
        } else {
            "0"
        };
        conn.execute_batch(&format!(
            "INSERT OR IGNORE INTO main.repo_marker(\
                 repo, last_head, last_indexed_at, last_mined_commit, root_path, archived) \
             SELECT repo, last_head, last_indexed_at, {mined_expr}, {root_expr}, {archived_expr} \
             FROM old.repo_marker;"
        ))?;
    }
    Ok(())
}

/// Copy the FTS5 + vec0 virtual tables. These may not support
/// `INSERT INTO … SELECT *` from an attached DB in all sqlite-vec versions,
/// so each row is copied via named columns: `code_fts` through the FTS5
/// content-table shape, `code_vec` as blobs tied to `symbol_id`.
fn copy_code_virtual_tables(conn: &Connection) -> Result<()> {
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
