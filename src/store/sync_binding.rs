//! First-push workspace binding and explicit `--allow-secret` overrides.
//! A memory bound to a workspace never moves by relabeling; secret overrides
//! are recorded here and shown in the console.

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

use crate::prelude::*;

/// One `sync_binding` row.
#[derive(Debug, Clone, Serialize)]
pub struct SyncBindingRow {
    /// 8-hex memory id.
    pub memory_id: String,
    /// Workspace that first accepted this memory.
    pub workspace_id: String,
    /// Redaction rule name when `--allow-secret` was used.
    pub secret_override_rule: Option<String>,
    /// ISO-8601 of the override, if any.
    pub secret_override_at: Option<String>,
}

/// Bind `memory_id` to `workspace_id` on first push (no-op when already bound).
pub fn bind_first(conn: &Connection, memory_id: &str, workspace_id: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO sync_binding(memory_id, workspace_id) VALUES(?1, ?2) \
         ON CONFLICT(memory_id) DO NOTHING",
        rusqlite::params![memory_id, workspace_id],
    )?;
    Ok(())
}

/// Record an explicit secret-scan override for `memory_id`.
pub fn allow_secret(
    conn: &Connection,
    memory_id: &str,
    workspace_id: &str,
    rule: &str,
    at: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO sync_binding(memory_id, workspace_id, secret_override_rule, secret_override_at) \
         VALUES(?1, ?2, ?3, ?4) \
         ON CONFLICT(memory_id) DO UPDATE SET \
           secret_override_rule = excluded.secret_override_rule, \
           secret_override_at = excluded.secret_override_at, \
           workspace_id = COALESCE(sync_binding.workspace_id, excluded.workspace_id)",
        rusqlite::params![memory_id, workspace_id, rule, at],
    )?;
    Ok(())
}

/// Load the binding for one memory, if any.
pub fn get(conn: &Connection, memory_id: &str) -> Result<Option<SyncBindingRow>> {
    conn.query_row(
        "SELECT memory_id, workspace_id, secret_override_rule, secret_override_at \
         FROM sync_binding WHERE memory_id = ?1",
        rusqlite::params![memory_id],
        |r| {
            Ok(SyncBindingRow {
                memory_id: r.get(0)?,
                workspace_id: r.get(1)?,
                secret_override_rule: r.get(2)?,
                secret_override_at: r.get(3)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

/// Whether a secret override is recorded for `memory_id`.
pub fn has_secret_override(conn: &Connection, memory_id: &str) -> Result<bool> {
    let row = get(conn, memory_id)?;
    Ok(row.is_some_and(|r| r.secret_override_rule.is_some()))
}

#[cfg(test)]
#[path = "tests/sync_binding.rs"]
mod tests;
