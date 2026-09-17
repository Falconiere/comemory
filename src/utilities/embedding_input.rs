//! Pure decoding of caller-supplied embedding vectors.
//!
//! Two input shapes are supported:
//!
//! - `--vector` — a comma-separated float list ([`parse_csv`]).
//! - `--vector-stdin` — a JSON `{ "embedding": [..] }` payload
//!   ([`parse_payload`]).
//!
//! Nothing here touches the process: reading stdin lives next door in
//! `crate::utilities::vector_stdin`, so the embed-command shell-out
//! ([`crate::utilities::embed`]) can decode a payload it captured from a child
//! process without pulling in the CLI's stdin acquisition (#166).

use serde::Deserialize;

use crate::prelude::*;

/// JSON payload accepted by `--vector-stdin` and emitted by the configured
/// embed command.
///
/// `deny_unknown_fields` rejects stray keys so callers notice schema drift
/// immediately rather than silently passing bad payloads.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
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

/// Parse a JSON `{ "embedding": [..] }` payload string into its inner vector.
/// Shared by `crate::utilities::vector_stdin::read_stdin_payload` and the
/// embed-command shell-out ([`crate::utilities::embed`]) so the payload shape
/// is decoded in exactly one place.
pub(crate) fn parse_payload(raw: &str) -> Result<Vec<f32>> {
    let payload: EmbeddingPayload = serde_json::from_str(raw.trim())?;
    Ok(payload.embedding)
}

#[cfg(test)]
#[path = "tests/embedding_input.rs"]
mod tests;
