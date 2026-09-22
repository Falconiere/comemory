//! The one rule both import wires apply to an arriving embedding, and the
//! record it leaves when it refuses one.
//!
//! An imported vector is only usable when it came from the model this engine
//! queries with, at the dimension its `vec0` table was built for. Refusing it
//! must never refuse the memory: the text is the memory, and a vector this
//! engine cannot compare against is worse than none at all.
//!
//! So a refusal stores the text and records the id in
//! [`crate::store::needs_embedding`] with the reason, which `comemory doctor`
//! reports and `comemory reembed` drains. The alternative — dropping the
//! vector silently, as the legacy path did — leaves an engine that looks
//! healthy and answers every semantic search a memory short.
//!
//! Shared by `exchange::import_write` and `replica::materialize` so the two
//! wires cannot reach different verdicts on the same bytes.

use crate::domains::sync::exchange::SyncVector;
use crate::prelude::*;
use crate::store::needs_embedding::{self, Pending, Reason};
use crate::store::{Connection, embed, schema_meta, vector};

/// What to do with one arriving embedding.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// Usable: store it against the memory.
    Store(Vec<f32>),
    /// Not usable: store the memory's text and record the backlog entry, with
    /// the reason and whatever arrived.
    Refuse {
        /// Why the vector is unusable.
        reason: Reason,
        /// The model that arrived, when one did.
        model: Option<String>,
        /// The dimension that arrived, when one did.
        dims: Option<i64>,
    },
}

/// Judge `wire` against this engine's embedder identity.
///
/// A vector whose model differs is refused as [`Reason::Model`], one whose
/// dimension differs as [`Reason::Dims`], and an absent one as
/// [`Reason::Absent`] — three distinguishable reasons, because the operator
/// action differs: re-embed locally, fix the peer's dimension, or wait for
/// the peer to start sending vectors at all.
///
/// # Errors
/// Propagates the `schema_meta` read, base64 decoding and the dim guard. A
/// malformed base64 body is a [`Error::BadRequest`], not a refusal: the peer
/// sent something that is not a vector, which is a protocol error rather than
/// an incompatible embedding.
pub fn decide(conn: &Connection, wire: Option<&SyncVector>) -> Result<Verdict> {
    let Some(wire) = wire else {
        return Ok(Verdict::Refuse {
            reason: Reason::Absent,
            model: None,
            dims: None,
        });
    };
    let model = schema_meta::memory_vector_model(conn)?;
    // Read from the `vec0` table itself rather than a second copy of the
    // literal: the width the vtab was built with is the only one this engine
    // can actually compare against.
    let dim = vector::dim_memory(conn)?;
    let refusal = |reason| Verdict::Refuse {
        reason,
        model: Some(wire.model.clone()),
        dims: Some(i64::from(wire.dims)),
    };
    if wire.model != model {
        return Ok(refusal(Reason::Model));
    }
    if wire.dims as usize != dim {
        return Ok(refusal(Reason::Dims));
    }
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &wire.f32)
        .map_err(|e| Error::BadRequest(format!("vector base64: {e}")))?;
    let values = embed::from_vec_blob(&bytes, dim)?;
    embed::guard_dim(&values, dim)?;
    Ok(Verdict::Store(values))
}

/// Carry out `verdict` for `memory_id` inside the caller's transaction.
///
/// Either outcome writes exactly one row and clears the other side, so a
/// memory is never both vectored and in the backlog: a later import that
/// brings a usable vector drains the backlog entry, and one that brings a
/// foreign vector drops the stale embedding it would otherwise keep being
/// searched by.
///
/// # Errors
/// Propagates SQLite failures.
pub fn apply(tx: &Connection, memory_id: &str, verdict: &Verdict, at: &str) -> Result<()> {
    match verdict {
        Verdict::Store(values) => {
            // `replace_memory` upserts, so a replay stores no second row.
            vector::replace_memory(tx, memory_id, values)?;
            needs_embedding::clear(tx, memory_id)
        }
        Verdict::Refuse {
            reason,
            model,
            dims,
        } => needs_embedding::record(
            tx,
            &Pending {
                memory_id: memory_id.to_string(),
                reason: *reason,
                model: model.clone(),
                dims: *dims,
            },
            at,
        ),
    }
}

#[cfg(test)]
#[path = "tests/vector_rule.rs"]
mod tests;
