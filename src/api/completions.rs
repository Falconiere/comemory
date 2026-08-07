//! `api::completions::{Request, run}` — the shared middle of `comemory
//! completions <shell>` / `GET /api/v1/completions?shell=`: generate a
//! shell completion script for `Cli` into an owned buffer. Moved out of
//! `cli::completions::run` (Binding Rule 1) — the CLI still writes the
//! result to stdout unchanged (byte-identical, AC-13). [`run`] never
//! touches stdout, generating into a `Vec<u8>` buffer instead so the same
//! bytes can also serve as an HTTP response body.
//!
//! Conn-free: `comemory completions` has no DB at all, so `run` never
//! calls `Ctx::conn`.

use std::str::FromStr;

use clap::CommandFactory;
use clap_complete::{Shell, generate};
use serde::Deserialize;

use crate::api::Ctx;
use crate::cli::Cli;
use crate::prelude::*;

/// `comemory completions` / `GET /api/v1/completions` request.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Shell to emit a completion script for: `bash`, `zsh`, `fish`,
    /// `powershell`, or `elvish`.
    pub shell: String,
}

/// Generate the completion script for `req.shell` as an owned `String`.
/// `ctx` is accepted only for the uniform `api::<cmd>::run` signature and
/// is never touched.
pub fn run(_ctx: &mut Ctx<'_>, req: Request) -> Result<String> {
    let shell = Shell::from_str(&req.shell)
        .map_err(|e| Error::BadRequest(format!("invalid shell {:?}: {e}", req.shell)))?;
    let mut cmd = Cli::command();
    let bin_name = cmd.get_name().to_string();
    let mut buf: Vec<u8> = Vec::new();
    generate(shell, &mut cmd, bin_name, &mut buf);
    String::from_utf8(buf).map_err(|e| Error::Other(format!("completion script not utf-8: {e}")))
}

#[cfg(test)]
#[path = "tests/completions.rs"]
mod tests;
