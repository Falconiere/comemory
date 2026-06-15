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

/// Read a JSON `{ "embedding": [..] }` payload from stdin and return the
/// inner vector. Caller is responsible for ensuring stdin is not also being
/// consumed for the memory body.
///
/// Reads at most 8 MiB from stdin before returning
/// `Error::Config("vector payload exceeds 8 MB limit")`, guarding against
/// callers accidentally piping a huge file into a vector slot.
pub(crate) fn read_stdin_payload() -> Result<Vec<f32>> {
    const LIMIT: u64 = 8 * 1024 * 1024;
    let mut buf = String::new();
    std::io::stdin()
        .take(LIMIT + 1)
        .read_to_string(&mut buf)
        .map_err(Error::Io)?;
    if buf.len() as u64 > LIMIT {
        return Err(Error::Config("vector payload exceeds 8 MB limit".into()));
    }
    parse_payload(&buf)
}

/// Parse a JSON `{ "embedding": [..] }` payload string into its inner vector.
/// Shared by [`read_stdin_payload`] and the TUI's embed-command shell-out
/// (`tui::embed`) so the payload shape is decoded in exactly one place.
pub(crate) fn parse_payload(raw: &str) -> Result<Vec<f32>> {
    let payload: EmbeddingPayload = serde_json::from_str(raw.trim())?;
    Ok(payload.embedding)
}

/// Resolve the optional caller-supplied vector from the `--vector` (CSV) and
/// `--vector-stdin` (JSON) flag pair. Returns `Ok(None)` when neither flag is
/// set so the FTS-only / lexical-only branches can proceed. Shared by every
/// subcommand that accepts a BYO-vector input (`save`, `search`, `context`)
/// so they cannot drift on what "no vector" means.
///
/// The two flags are mutually exclusive: passing both is rejected up front so
/// the caller doesn't get a silent winner (previously `--vector-stdin` would
/// silently override `--vector`, which is confusing when a script accidentally
/// sets both env-driven flags).
pub(crate) fn read_optional(
    vector_stdin: bool,
    vector_csv: Option<&str>,
) -> Result<Option<Vec<f32>>> {
    if vector_stdin && vector_csv.is_some() {
        return Err(Error::Config(
            "--vector and --vector-stdin are mutually exclusive; pick one".into(),
        ));
    }
    if vector_stdin {
        return Ok(Some(read_stdin_payload()?));
    }
    if let Some(raw) = vector_csv {
        return Ok(Some(parse_csv(raw)?));
    }
    Ok(None)
}
