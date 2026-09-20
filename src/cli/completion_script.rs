//! `cli::completion_script::{Request, run}` — the shared middle of `comemory
//! completions <shell>` / `GET /api/v1/completions?shell=`: generate a
//! shell completion script for [`Cli`] into an owned buffer. [`run`] never
//! touches stdout, generating into a `Vec<u8>` buffer instead so the same
//! bytes can also serve as an HTTP response body.
//!
//! Delivery-owned on purpose (#166): generating a completion script *is* clap
//! work, so it belongs beside the clap definition it reflects rather than in
//! a command core that would then have to import `cli::Cli`.
//!
//! Conn-free: `comemory completions` has no DB at all, so `run` never
//! calls `Ctx::conn`.

use std::str::FromStr;

use clap::CommandFactory;
use clap_complete::{Shell, generate};
use serde::Deserialize;

use crate::cli::Cli;
use crate::prelude::*;
use crate::utilities::context::Ctx;

/// `comemory completions` / `GET /api/v1/completions` request.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Shell to emit a completion script for: `bash`, `zsh`, `fish`,
    /// `powershell`, or `elvish`.
    pub shell: String,
}

/// Generate the completion script for `req.shell` as an owned `String`.
/// `ctx` is accepted only for the uniform command-core `run` signature and
/// is never touched.
pub fn run(_ctx: &mut Ctx<'_>, req: Request) -> Result<String> {
    let shell = Shell::from_str(&req.shell)
        .map_err(|e| Error::BadRequest(format!("invalid shell {:?}: {e}", req.shell)))?;
    generate_for(shell)
}

/// Generate a completion script for a parsed shell value.
pub(crate) fn generate_for(shell: Shell) -> Result<String> {
    let mut cmd = Cli::command();
    let bin_name = cmd.get_name().to_string();
    let mut buf: Vec<u8> = Vec::new();
    generate(shell, &mut cmd, bin_name, &mut buf);
    String::from_utf8(buf).map_err(|e| Error::Other(format!("completion script not utf-8: {e}")))
}

#[cfg(test)]
#[path = "tests/completion_script.rs"]
mod tests;
