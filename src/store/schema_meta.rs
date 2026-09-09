//! Single-key `schema_meta` writers not already owned by `migrate` (the
//! migration marker set) or `vector` (the memory/code vector dim guards).
//!
//! Moved out of `config::sync::apply_embed_model` (spec
//! `docs/toolu/specs/2026-09-07-store-layer-chokepoint-design.md`).

use rusqlite::{Connection, params};

use crate::prelude::*;

/// Upsert `schema_meta.memory_vector_model` — the embedder model id
/// [`crate::config::sync::EmbedConfig`] records, surfaced by `comemory
/// doctor`.
pub fn set_memory_vector_model(conn: &Connection, model: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO schema_meta(key, value) VALUES('memory_vector_model', ?1) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![model],
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/schema_meta.rs"]
mod tests;
