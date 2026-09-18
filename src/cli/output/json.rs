//! Single-line JSON output for machine-readable CLI modes. Writes
//! `serde_json::to_string(value)` to stdout followed by a newline.

use std::io::Write as _;

use serde::Serialize;

use crate::prelude::*;

/// Serialize `v` as a single-line JSON string and write it to stdout with a
/// trailing newline. Locks stdout once for the duration of the write so output
/// is not interleaved with other writers.
pub fn write<T: Serialize>(v: &T) -> Result<()> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{}", serde_json::to_string(v)?)?;
    Ok(())
}
