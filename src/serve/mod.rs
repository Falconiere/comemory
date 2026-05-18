//! Local HTTP viewer for the kuzu property graph.
//!
//! Exposes `qwick-memory graph serve`. Read-only, loopback-only, embedded
//! frontend served from `frontend/` via `rust-embed`.

pub mod state;

pub use state::ServerState;
