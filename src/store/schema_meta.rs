//! Single-key `schema_meta` writers not already owned by `migrate` (the
//! migration marker set) or `vector` (the memory/code vector dim guards),
//! plus [`get`]/[`upsert`] — the generic arbitrary-key read/write behind
//! `cli::lazy_reindex`'s per-repo debounce marker.
//!
//! Moved out of `config::sync::apply_embed_model` (spec
//! `docs/toolu/specs/2026-09-07-store-layer-chokepoint-design.md`).

use rusqlite::{Connection, OptionalExtension, params};

use crate::prelude::*;

/// Upsert `schema_meta.memory_vector_model` — the embedder model id
/// [`crate::config::sync::EmbedConfig`] records, surfaced by `comemory
/// doctor`.
pub fn set_memory_vector_model(conn: &Connection, model: &str) -> Result<()> {
    upsert(conn, "memory_vector_model", model)
}

/// The stored `schema_meta.version` value. Errors (including a missing row)
/// propagate via `?` exactly as the bare query did before this moved out of
/// `api::doctor` — the key is written unconditionally by
/// `store::migrate::set_version`, so a missing row means a database
/// `migrate::run` never touched, not a normal "not found" a caller should
/// branch on.
pub fn version(conn: &Connection) -> Result<String> {
    Ok(conn.query_row(
        "SELECT value FROM schema_meta WHERE key = 'version'",
        [],
        |r| r.get(0),
    )?)
}

/// Read the `schema_meta` value stored under `key`, or `None` when absent.
pub(crate) fn get(conn: &Connection, key: &str) -> Result<Option<String>> {
    conn.query_row("SELECT value FROM schema_meta WHERE key = ?1", [key], |r| {
        r.get(0)
    })
    .optional()
    .map_err(Error::Sqlite)
}

/// Upsert an arbitrary `schema_meta(key, value)` pair, overwriting any
/// existing value for `key`.
pub(crate) fn upsert(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO schema_meta(key, value) VALUES(?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/schema_meta.rs"]
mod tests;
