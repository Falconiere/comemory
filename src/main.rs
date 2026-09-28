//! Binary entry point: parse args, run the command, and map a returned
//! [`Error`] to a sysexits code (design §6.4) through
//! [`comemory::utilities::exit_code::exit_code`], which owns the table.

use std::io::Write as _;

use clap::Parser;

use comemory::cli::{Cli, run};
use comemory::errors::Error;
use comemory::utilities::exit_code::exit_code;

#[tokio::main]
async fn main() {
    // Diagnostics go to stderr: stdout is a data channel for `--json` and the
    // whole JSON-RPC stream for `comemory mcp`, so a `RUST_LOG` line there
    // would corrupt what the caller parses.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
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
