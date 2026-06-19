//! Binary entry point. Parses CLI args, runs the dispatched command, and
//! maps any returned [`Error`] to a sysexits-style exit code so shell users
//! and supervisors can react meaningfully (mapping per design §6.4):
//!
//! - 0  — success
//! - 64 — `EX_USAGE` (`NotFound`, `Usage`)
//! - 65 — `EX_DATAERR` (`Yaml`, `Json`, `Toml`, `Frontmatter`, `VecDimMismatch`)
//! - 69 — `EX_UNAVAILABLE` (`Unavailable`)
//! - 70 — `EX_SOFTWARE` (`Sqlite`, `Migration`, `Ast`, `Git`, `Forbidden`,
//!   `BadRequest`, `Other`)
//! - 74 — `EX_IOERR` (`Io`)
//! - 78 — `EX_CONFIG` (`Config`)

use std::io::Write as _;

use clap::Parser;

use comemory::cli::{Cli, run};
use comemory::errors::Error;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let cli = Cli::parse();
    let code = match run(cli).await {
        Ok(()) => 0,
        Err(e) => report(e),
    };
    std::process::exit(code);
}

/// Print `error: <message>` for `err` to stderr and return its sysexits exit
/// code (mapping per design §6.4; see the module header). The printed message
/// is the error's `Display` for every variant except [`Error::Other`], whose
/// `other:` prefix is dropped so the bare message reaches the user.
fn report(err: Error) -> i32 {
    let mut sink = std::io::stderr().lock();
    let _ = match &err {
        Error::Other(msg) => writeln!(sink, "error: {msg}"),
        _ => writeln!(sink, "error: {err}"),
    };
    exit_code(&err)
}

/// Map an [`Error`] to its sysexits-style exit code (mapping per design §6.4).
fn exit_code(err: &Error) -> i32 {
    match err {
        Error::Io(_) => 74,
        Error::Config(_) => 78,
        Error::Unavailable(_) => 69,
        Error::NotFound(_) | Error::Usage(_) => 64,
        Error::Yaml(_)
        | Error::Json(_)
        | Error::Toml(_)
        | Error::VecDimMismatch { .. }
        | Error::Frontmatter(_) => 65,
        Error::Sqlite(_)
        | Error::Ast(_)
        | Error::Git(_)
        | Error::Migration(_)
        | Error::Forbidden(_)
        | Error::BadRequest(_)
        | Error::Other(_) => 70,
    }
}
