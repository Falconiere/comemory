//! Shared parsing for caller-supplied embedding vectors used by both
//! `comemory save` and `comemory search`.
//!
//! Two input shapes are supported:
//!
//! - `--vector` — comma-separated floats parsed in-place.
//! - `--vector-stdin` — JSON `{ "embedding": [..] }` read from stdin.
//!
//! Keeping both call sites pointed at this module avoids duplicating the
//! CSV/JSON parsing logic.

use std::io::Read;

use serde::Deserialize;

use crate::prelude::*;

/// JSON payload accepted by `--vector-stdin`.
///
/// `pub(crate)` because both `cli::save` and `cli::search` deserialize this
/// shape; the visibility lets either subcommand reuse it without exposing
/// the type in the public crate surface.
#[derive(Deserialize)]
pub(crate) struct EmbeddingPayload {
    /// Dense vector handed in by the caller.
    pub(crate) embedding: Vec<f32>,
}

/// Parse a comma-separated float list. Whitespace around each component is
/// stripped before parsing so callers can write `1.0, 2.0, 3.0`.
pub(crate) fn parse_csv(raw: &str) -> Result<Vec<f32>> {
    raw.split(',')
        .map(|s| s.trim().parse::<f32>())
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| Error::Config(format!("--vector parse: {e}")))
}

/// Read a JSON `{ "embedding": [..] }` payload from stdin and return the
/// inner vector. Caller is responsible for ensuring stdin is not also being
/// consumed for the memory body.
pub(crate) fn read_stdin_payload() -> Result<Vec<f32>> {
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .map_err(Error::Io)?;
    let payload: EmbeddingPayload = serde_json::from_str(buf.trim())?;
    Ok(payload.embedding)
}
