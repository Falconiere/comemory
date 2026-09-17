//! Acquire a caller-supplied embedding vector from the process's flags and
//! standard input.
//!
//! Split from `crate::utilities::embedding_input` (#166) so payload decoding
//! stays pure: this file is the only one that reads `std::io::stdin`, and it is
//! what `save`, `search`, `search-code`, `context` and `find` call once they
//! have already validated everything that must be validated before stdin is
//! consumed.

use std::io::Read;

use crate::prelude::*;
use crate::utilities::embedding_input::{parse_csv, parse_payload};

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

/// Resolve the optional caller-supplied vector from the `--vector` (CSV) and
/// `--vector-stdin` (JSON) flag pair. Returns `Ok(None)` when neither flag is
/// set so the FTS-only / lexical-only branches can proceed. Shared by every
/// subcommand that accepts a BYO-vector input (`save`, `search`,
/// `search-code`, `context`, `find`) so they cannot drift on what "no vector"
/// means.
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
