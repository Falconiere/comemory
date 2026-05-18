//! Deterministic retrieval pipeline over the memory layer: query routing,
//! hybrid (vector + future FTS) search, ranking helpers, and the corrective
//! fallback signal. Each submodule owns one concern; this `mod.rs` only
//! re-exports the shapes callers (CLI, MCP) need at the boundary.

pub mod bundle;
pub mod corrective;
pub mod hybrid;
pub mod rank;
pub mod router;

pub use bundle::{Bundle, CitedHit};
pub use router::{classify, Route};
