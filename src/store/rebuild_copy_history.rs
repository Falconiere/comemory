//! The run-history half of [`super::rebuild_copy`]'s preservation copy —
//! `eval_runs` (v14, plus v15's `discarded` flag), `gc_runs` (v14),
//! `index_runs` (v15), the v16 cloud-sync tables (`sync_log` / `sync_state` /
//! `sync_binding`) and the v22 `replica-v1` journal. History is exactly what
//! markdown cannot reconstruct: a rebuild that dropped it would erase every
//! recorded eval, gc, and index run (and re-offer every discarded knob
//! proposal), would reset sync cursors / bindings, and would reissue
//! replication sequences a peer already holds a receipt for.

use crate::prelude::*;
use crate::store::Connection;
use crate::store::rebuild_copy::{old_column_exists, old_table_exists};

/// Copy the three run-history tables from the attached `old` database. A
/// pre-v15 `eval_runs` has no `discarded` column, so it is synthesized as
/// `0` (the migration's own default); a pre-v14 source has none of these
/// tables and each block is skipped. Also copies the v16 sync tables when
/// present.
pub(crate) fn copy_history_tables(conn: &Connection) -> Result<()> {
    if old_table_exists(conn, "eval_runs")? {
        let discarded_expr = if old_column_exists(conn, "eval_runs", "discarded")? {
            "discarded"
        } else {
            "0"
        };
        conn.execute_batch(&format!(
            "INSERT OR IGNORE INTO main.eval_runs(\
                 id, kind, at, golden_pairs, k, recall, mrr, knobs, applied, discarded) \
             SELECT id, kind, at, golden_pairs, k, recall, mrr, knobs, applied, {discarded_expr} \
             FROM old.eval_runs;"
        ))?;
    }
    if old_table_exists(conn, "gc_runs")? {
        conn.execute_batch(
            "INSERT OR IGNORE INTO main.gc_runs(\
                 id, at, removed, log_rows, event_rows, bytes_freed) \
             SELECT id, at, removed, log_rows, event_rows, bytes_freed \
             FROM old.gc_runs;",
        )?;
    }
    if old_table_exists(conn, "index_runs")? {
        conn.execute_batch(
            "INSERT OR IGNORE INTO main.index_runs(\
                 id, repo, root_path, mode, started_at, finished_at, duration_ms, \
                 files_indexed, symbols, outcome, error) \
             SELECT id, repo, root_path, mode, started_at, finished_at, duration_ms, \
                 files_indexed, symbols, outcome, error \
             FROM old.index_runs;",
        )?;
    }
    copy_sync_tables(conn)?;
    Ok(())
}

/// Copy the preserved cloud-sync and `replica-v1` tables the attached DB has.
///
/// One pass over a list rather than a function per family: each entry is the
/// table plus the column list to carry over, and a table the source predates
/// is skipped.
fn copy_sync_tables(conn: &Connection) -> Result<()> {
    for (table, columns) in PRESERVED {
        copy_table(conn, table, columns)?;
    }
    Ok(())
}

/// `(table, columns)` for every row set a rebuild carries over from the old
/// database. `replica_stream` leads the replica group because the epoch is
/// what makes the positions after it meaningful.
const PRESERVED: &[(&str, &str)] = &[
    ("sync_log", "seq, op, memory_id, content_hash, at, origin"),
    (
        "sync_state",
        "workspace_id, api_url, pulled_seq, pushed_seq, last_sync_at",
    ),
    (
        "sync_binding",
        "memory_id, workspace_id, secret_override_rule, secret_override_at",
    ),
    ("replica_stream", "id, epoch, created_at"),
    (
        "replica_payload",
        "digest, entity_kind, schema_version, bytes, byte_len, created_at, redacted_at",
    ),
    (
        "replica_feed",
        "sequence, epoch, entity_kind, entity_key, op, payload_digest, schema_version, \
         operation_id, origin, repository, at",
    ),
    (
        "replica_revision",
        "entity_kind, entity_key, sequence, payload_digest, deleted, deleted_sequence, \
         updated_at",
    ),
    (
        "replica_operation",
        "operation_id, entity_kind, entity_key, op, payload_digest, schema_version, \
         repository, observed_sequence, state, upstream_sequence, disposition, attempts, \
         last_error, created_at, updated_at",
    ),
    (
        "replica_receipt",
        "operation_id, epoch, sequence, disposition, payload_digest, reason, accepted_at",
    ),
    (
        "replica_cursor",
        "workspace_id, api_url, stream_epoch, applied_sequence, updated_at",
    ),
];

/// Copy one table's columns from the attached `old` database, if it has it.
///
/// `replica_stream` is replaced rather than merged: the fresh database minted
/// its own epoch at migration time, and keeping that one would tell every peer
/// its cursor belongs to a stream that no longer exists.
fn copy_table(conn: &Connection, table: &str, columns: &str) -> Result<()> {
    if !old_table_exists(conn, table)? {
        return Ok(());
    }
    if table == "replica_stream" {
        conn.execute_batch("DELETE FROM main.replica_stream;")?;
    }
    conn.execute_batch(&format!(
        "INSERT OR IGNORE INTO main.{table}({columns}) SELECT {columns} FROM old.{table};"
    ))?;
    Ok(())
}
