//! The schema-migration tool behind `just migration`, `just migration-journal`
//! and `just migration-adopt` — a dev-only target, never part of the release
//! binary.
//!
//! ```text
//! cargo run -q --example migrations -- generate <name>   # struct diff → migrations/NNNN_<name>.sql
//! cargo run -q --example migrations -- journal <path>    # journal a hand-written migration
//! cargo run -q --example migrations -- adopt             # rewrite the newest snapshot from the registry
//! ```
//!
//! `generate` is toolu-orm's `run_generate` over `store::schema::registry()`:
//! it diffs the declared structs against the newest `*.snapshot.json`, and
//! when something changed writes the numbered SQL file, its snapshot, and a
//! `_journal.json` entry carrying the file's SHA-256. `journal` is the
//! hand-SQL twin — a migration for a table the registry does not declare —
//! and refuses a name already in the journal (exit 64) so `run_generate`'s
//! `max(NNNN) + 1` numbering can never collide silently. `adopt` restates the
//! newest snapshot from the registry without writing SQL: the step after a
//! previously hand-managed table becomes declared, and how the baseline was
//! seeded. Paths resolve from `CARGO_MANIFEST_DIR`, so the cwd is irrelevant.
//!
//! Runtime apply is not here: `store::migrate::run` applies every file with
//! `execute_batch` (the `--> statement-breakpoint` lines are SQL comments),
//! keyed by `schema_meta` markers — see `docs/guides/schema-migrations.md`.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use comemory::errors::Error;
use comemory::store::schema::registry;
use comemory::store::schema_journal::{self, journal_file};
use toolu_orm::core::dialect::Dialect;
use toolu_orm_cli::generate::run_generate;

const USAGE: &str =
    "usage: cargo run -q --example migrations -- generate <name> | journal <path> | adopt";

/// `sysexits.h` `EX_USAGE`, the same code the `comemory` binary uses.
const EX_USAGE: u8 = 64;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(message) => {
            let _ = writeln!(std::io::stderr(), "migrations: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<ExitCode, String> {
    let dir = migrations_dir();
    match args {
        [cmd, name] if cmd == "generate" => generate(&dir, name),
        [cmd, path] if cmd == "journal" => journal(&dir, Path::new(path)),
        [cmd] if cmd == "adopt" => adopt(&dir),
        _ => {
            let _ = writeln!(std::io::stderr(), "{USAGE}");
            Ok(ExitCode::from(EX_USAGE))
        }
    }
}

/// `<crate root>/migrations` — toolu-orm's `migrations_dir`.
fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations")
}

/// `dir` as the `&str` toolu-orm's file APIs take.
fn dir_str(dir: &Path) -> Result<&str, String> {
    dir.to_str()
        .ok_or_else(|| format!("{} is not valid UTF-8", dir.display()))
}

fn say(line: &str) -> Result<(), String> {
    writeln!(std::io::stdout(), "{line}").map_err(|e| format!("stdout: {e}"))
}

fn generate(dir: &Path, name: &str) -> Result<ExitCode, String> {
    let written = run_generate(&registry(), dir_str(dir)?, name, Dialect::Sqlite)
        .map_err(|e| format!("generate: {e}"))?;
    match written {
        Some(file) => say(&format!(
            "wrote migrations/{file} (+ snapshot + journal entry); now wire it into \
             store::migrate::list::MIGRATIONS and migrations/README.md"
        ))?,
        None => say("no schema change")?,
    }
    Ok(ExitCode::SUCCESS)
}

fn journal(dir: &Path, path: &Path) -> Result<ExitCode, String> {
    match journal_file(dir, path) {
        Ok(entry) => {
            say(&format!("journaled {} {}", entry.name, entry.hash))?;
            Ok(ExitCode::SUCCESS)
        }
        Err(e) => usage_or_fail(e),
    }
}

fn adopt(dir: &Path) -> Result<ExitCode, String> {
    match schema_journal::adopt(dir) {
        Ok(written) => {
            let name = written
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            say(&format!(
                "adopted the declared schema into migrations/{name}"
            ))?;
            Ok(ExitCode::SUCCESS)
        }
        Err(e) => usage_or_fail(e),
    }
}

/// `Error::Usage` is the caller's mistake (a duplicate journal name, an empty
/// journal): report it and exit `EX_USAGE`; anything else is a failure.
fn usage_or_fail(e: Error) -> Result<ExitCode, String> {
    match e {
        Error::Usage(message) => {
            let _ = writeln!(std::io::stderr(), "migrations: {message}");
            Ok(ExitCode::from(EX_USAGE))
        }
        other => Err(other.to_string()),
    }
}
