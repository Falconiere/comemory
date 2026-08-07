//! Pruning: stale-memory detectors plus soft-delete + gc primitives.
//!
//! Each submodule exposes a `detect` function that returns the ids (or paths)
//! that are candidates for removal. The CLI surface (`src/cli/prune.rs` and
//! `src/cli/gc.rs`) is responsible for turning those candidates into actions
//! (soft-delete via [`crate::memory::MemoryStore::delete`], or filesystem
//! purges from `memories/.trash/`).
//!
//! Detection is read-only and side-effect free; `--apply` is what mutates the
//! store.

pub mod low_value;
pub mod orphans;
pub mod stale_code;
