//! Binary entry point. Parses CLI args, runs the dispatched command, and
//! maps any returned [`Error`] to a sysexits-style exit code so shell users
//! and supervisors can react meaningfully (mapping per design §6.4):
//!
//! - 0  — success
//! - 64 — `EX_USAGE` (`NotFound`)
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
        Err(Error::Io(e)) => {
            let mut err = std::io::stderr().lock();
            let _ = writeln!(err, "error: io: {e}");
            74
        }
        Err(Error::Sqlite(e)) => {
            let mut err = std::io::stderr().lock();
            let _ = writeln!(err, "error: sqlite: {e}");
            70
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
        Err(Error::Ast(msg)) => {
            let mut err = std::io::stderr().lock();
            let _ = writeln!(err, "error: ast: {msg}");
            70
        }
        Err(Error::Git(e)) => {
            let mut err = std::io::stderr().lock();
            let _ = writeln!(err, "error: git: {e}");
            70
        }
        Err(Error::Migration(msg)) => {
            let mut err = std::io::stderr().lock();
            let _ = writeln!(err, "error: schema migration failed: {msg}");
            70
        }
        Err(e @ Error::VecDimMismatch { .. }) => {
            let mut err = std::io::stderr().lock();
            let _ = writeln!(err, "error: {e}");
            65
        }
        Err(Error::Frontmatter(msg)) => {
            let mut err = std::io::stderr().lock();
            let _ = writeln!(err, "error: invalid frontmatter: {msg}");
            65
        }
        Err(Error::NotFound(msg)) => {
            let mut err = std::io::stderr().lock();
            let _ = writeln!(err, "error: memory not found: {msg}");
            64
        }
        Err(Error::Config(msg)) => {
            let mut err = std::io::stderr().lock();
            let _ = writeln!(err, "error: config: {msg}");
            78
        }
        Err(Error::Forbidden(msg)) => {
            let mut err = std::io::stderr().lock();
            let _ = writeln!(err, "error: forbidden: {msg}");
            70
        }
        Err(Error::BadRequest(msg)) => {
            let mut err = std::io::stderr().lock();
            let _ = writeln!(err, "error: bad request: {msg}");
            70
        }
        Err(Error::Unavailable(msg)) => {
            let mut err = std::io::stderr().lock();
            let _ = writeln!(err, "error: unavailable: {msg}");
            69
        }
        Err(Error::Other(msg)) => {
            let mut err = std::io::stderr().lock();
            let _ = writeln!(err, "error: {msg}");
            70
        }
    };
    std::process::exit(code);
}
