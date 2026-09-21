//! The three plain config sections with no overlay logic of their own:
//! `[git]`, `[embeddings]` and `[output]`.
//!
//! Split out of `file.rs` so Binding Rule 3 (<= 300 code lines) stays green;
//! each is a value struct `Config` holds, overlaid field-by-field by
//! `Config::apply_file` rather than by an `apply` of its own.

use serde::{Deserialize, Serialize};

/// Best-effort git auto-sync of the markdown source of truth.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitConfig {
    /// Commit + push after a save when enabled. Env: `COMEMORY_GIT_AUTO_SYNC`.
    pub auto_sync: bool,
    /// Remote pushed to by the auto-sync; empty means the git default.
    pub remote: String,
}

/// Operator-visible record of the embedders that produced the vectors.
///
/// Reporting-only: comemory is BYO-vector and never runs an embedder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingsConfig {
    /// Model name recorded for `memory_vec` vectors.
    pub memory_model: String,
    /// Model name recorded for `code_vec` vectors.
    pub code_model: String,
}

/// Emitter defaults shared by every subcommand.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputConfig {
    /// Emit JSON instead of the TTY renderer. Overridden by `--json`.
    pub json: bool,
    /// Colour policy for the TTY renderer: `auto`, `always`, or `never`.
    pub color: String,
}
