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
    copy_replica_tables(conn)?;
    Ok(())
}

/// Copy the v22 `replica-v1` journal when the attached DB has it.
///
/// A rebuild re-derives the database from markdown, but a sequence this
/// engine already handed to a peer cannot be re-derived: reissuing it would
/// turn every replayed operation into a second effect, and dropping the
/// receipts would answer a replay with a fresh acceptance. The stream epoch
/// is copied for the same reason — a rebuild is not a replaced stream, and a
/// peer's cursor must stay valid across it. Staged parts are deliberately not
/// copied: an upload that never activated published nothing.
fn copy_replica_tables(conn: &Connection) -> Result<()> {
    if !old_table_exists(conn, "replica_feed")? {
        return Ok(());
    }
    conn.execute_batch(
        "DELETE FROM main.replica_stream; \
         INSERT INTO main.replica_stream(id, epoch, created_at) \
         SELECT id, epoch, created_at FROM old.replica_stream; \
         INSERT OR IGNORE INTO main.replica_payload(\
             digest, entity_kind, schema_version, bytes, byte_len, created_at, redacted_at) \
         SELECT digest, entity_kind, schema_version, bytes, byte_len, created_at, redacted_at \
         FROM old.replica_payload; \
         INSERT OR IGNORE INTO main.replica_feed(\
             sequence, epoch, entity_kind, entity_key, op, payload_digest, schema_version, \
             operation_id, origin, repository, at) \
         SELECT sequence, epoch, entity_kind, entity_key, op, payload_digest, schema_version, \
             operation_id, origin, repository, at FROM old.replica_feed; \
         INSERT OR IGNORE INTO main.replica_revision(\
             entity_kind, entity_key, sequence, payload_digest, deleted, deleted_sequence, \
             updated_at) \
         SELECT entity_kind, entity_key, sequence, payload_digest, deleted, deleted_sequence, \
             updated_at FROM old.replica_revision; \
         INSERT OR IGNORE INTO main.replica_operation(\
             operation_id, entity_kind, entity_key, op, payload_digest, schema_version, \
             repository, observed_sequence, state, upstream_sequence, disposition, attempts, \
             last_error, created_at, updated_at) \
         SELECT operation_id, entity_kind, entity_key, op, payload_digest, schema_version, \
             repository, observed_sequence, state, upstream_sequence, disposition, attempts, \
             last_error, created_at, updated_at FROM old.replica_operation; \
         INSERT OR IGNORE INTO main.replica_receipt(\
             operation_id, epoch, sequence, disposition, payload_digest, reason, accepted_at) \
         SELECT operation_id, epoch, sequence, disposition, payload_digest, reason, accepted_at \
         FROM old.replica_receipt; \
         INSERT OR IGNORE INTO main.replica_cursor(\
             workspace_id, api_url, stream_epoch, applied_sequence, updated_at) \
         SELECT workspace_id, api_url, stream_epoch, applied_sequence, updated_at \
         FROM old.replica_cursor;",
    )?;
    Ok(())
}

/// Copy `sync_log` / `sync_state` / `sync_binding` when the attached DB has
/// them (post-v16). Skipped on older sources.
fn copy_sync_tables(conn: &Connection) -> Result<()> {
    if old_table_exists(conn, "sync_log")? {
        conn.execute_batch(
            "INSERT OR IGNORE INTO main.sync_log(\
                 seq, op, memory_id, content_hash, at, origin) \
             SELECT seq, op, memory_id, content_hash, at, origin FROM old.sync_log;",
        )?;
    }
    if old_table_exists(conn, "sync_state")? {
        conn.execute_batch(
            "INSERT OR IGNORE INTO main.sync_state(\
                 workspace_id, api_url, pulled_seq, pushed_seq, last_sync_at) \
             SELECT workspace_id, api_url, pulled_seq, pushed_seq, last_sync_at \
             FROM old.sync_state;",
        )?;
    }
    if old_table_exists(conn, "sync_binding")? {
        conn.execute_batch(
            "INSERT OR IGNORE INTO main.sync_binding(\
                 memory_id, workspace_id, secret_override_rule, secret_override_at) \
             SELECT memory_id, workspace_id, secret_override_rule, secret_override_at \
             FROM old.sync_binding;",
        )?;
    }
    Ok(())
}
