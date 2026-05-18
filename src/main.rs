//! Binary entry point. Parses CLI args, runs the dispatched command, and
//! maps any returned [`Error`] to a sysexits-style exit code so shell users
//! and supervisors can react meaningfully:
//!
//! - 0   — success
//! - 65  — `EX_DATAERR` (malformed yaml / json / toml on disk or in args)
//! - 70  — `EX_SOFTWARE` (internal logic / wrapped foreign errors)
//! - 74  — `EX_IOERR` (filesystem, network, sub-process I/O)

use std::io::Write as _;

use clap::Parser;

use qwick::cli::{run, Cli};
use qwick::errors::Error;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let cli = Cli::parse();
    let code = match run(cli).await {
        Ok(()) => 0,
        Err(Error::Io(e)) => {
            let mut err = std::io::stderr().lock();
            let _ = writeln!(err, "error: io: {e}");
            74
        }
        Err(Error::Yaml(e)) => {
            let mut err = std::io::stderr().lock();
            let _ = writeln!(err, "error: yaml: {e}");
            65
        }
        Err(Error::Json(e)) => {
            let mut err = std::io::stderr().lock();
            let _ = writeln!(err, "error: json: {e}");
            65
        }
        Err(Error::Toml(e)) => {
            let mut err = std::io::stderr().lock();
            let _ = writeln!(err, "error: toml: {e}");
            65
        }
        Err(Error::Lance(msg)) => {
            let mut err = std::io::stderr().lock();
            let _ = writeln!(err, "error: lancedb: {msg}");
            70
        }
        Err(Error::Other(msg)) => {
            let mut err = std::io::stderr().lock();
            let _ = writeln!(err, "error: {msg}");
            70
        }
    };
    std::process::exit(code);
}
