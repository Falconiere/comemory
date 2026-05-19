//! Local HTTP viewer for the kuzu property graph.
//!
//! Exposes `qwick-memory graph serve`. Read-only, loopback-only, embedded
//! frontend served from `frontend/` via `rust-embed`.

pub mod assets;
pub mod dto;
pub mod error;
pub mod handlers;
pub mod router;
pub mod state;

pub use error::ApiError;
pub use router::run;
pub use state::ServerState;
