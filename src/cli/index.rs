//! `comemory index <PATH>...` — register one or more files or directories
//! as document sources and run their synchronous initial reconcile:
//! discover → classify → extract/write each candidate, then tombstone any
//! previously-indexed file an authoritative scan no longer sees. See the
//! design spec's "Durable source registry" and "Index lifecycle and
//! freshness" sections.

use std::collections::HashSet;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use clap::Args as ClapArgs;
use rusqlite::Connection;
use serde::Serialize;

use crate::cli::{load_config, resolve_data_dir};
use crate::config::paths::Paths;
use crate::document::writer::{self, UpdateOutcome};
use crate::output::{json, tty};
use crate::prelude::*;
use crate::source::SourceEntry;
use crate::source::discover::{self, Candidate};
use crate::source::mirror;
use crate::source::registry::Registry;
use crate::store::connection;

const EXAMPLES: &str = "\
Examples:
  # Register and index a directory of notes
  comemory index ~/notes

  # Register a single file with an explicit repo label
  comemory index README.md --repo comemory

  # Register two sources in one run, failing loudly on any per-file error
  comemory index ~/notes ~/docs --strict";

/// Arguments to `comemory index`.
#[derive(ClapArgs, Debug)]
#[command(after_help = EXAMPLES)]
pub struct Args {
    /// One or more files or directories to register as document sources.
    #[arg(required = true, value_name = "PATH")]
    pub path: Vec<PathBuf>,
    /// Repository label attached to every document under each registered
    /// source. Defaults to the basename of the nearest enclosing git
    /// worktree, when one exists.
    #[arg(long)]
    pub repo: Option<String>,
    /// Exit `65` (`EX_DATAERR`) when any per-file error (corrupt/unreadable/
    /// oversized) occurred, after every file has been attempted. Without
    /// this flag a partial-success reconcile still exits `0`.
    #[arg(long, default_value_t = false)]
    pub strict: bool,
}

/// One per-file diagnostic surfaced in a [`SourceReport`].
#[derive(Serialize, Debug)]
struct FileError {
    /// Path relative to the source root.
    path: String,
    /// `"too_large"` or `"error"`.
    kind: &'static str,
    /// The typed diagnostic message.
    message: String,
}

/// Reconcile outcome for one registered source.
#[derive(Serialize, Debug)]
struct SourceReport {
    source_id: String,
    canonical_path: String,
    repo: Option<String>,
    indexed: usize,
    unchanged: usize,
    too_large: usize,
    unsupported: usize,
    ignored: usize,
    removed: usize,
    errors: Vec<FileError>,
}

/// Full `comemory index` report: one [`SourceReport`] per registered path,
/// in invocation order.
#[derive(Serialize, Debug)]
struct Output {
    sources: Vec<SourceReport>,
}

/// Register every `a.path` entry and run its synchronous reconcile.
pub async fn run(a: Args, json_flag: bool, data_dir: Option<PathBuf>) -> Result<()> {
    let paths = Paths::new(resolve_data_dir(data_dir));
    paths.ensure_dirs()?;
    let cfg = load_config(&paths)?;
    let registry = Registry::new(paths.clone());
    let mut conn = connection::open(paths.db_path())?;

    let mut reports = Vec::with_capacity(a.path.len());
    for raw_path in &a.path {
        let canonical = validate_input_path(raw_path)?;
        let repo = a.repo.clone().or_else(|| infer_repo_label(&canonical));
        let entry = registry.register(raw_path, repo)?;
        // Reconcile the SQLite mirror before indexing this entry so its
        // `source_roots` row (the FK parent of `source_files`) exists.
        mirror::reconcile(&conn, &registry.load()?)?;
        let report = reconcile_source(&mut conn, &entry, cfg.indexing.max_file_bytes, &paths)?;
        reports.push(report);
    }

    let strict_failed = a.strict && reports.iter().any(|r| !r.errors.is_empty());
    emit(json_flag, &Output { sources: reports })?;
    if strict_failed {
        return Err(Error::Document(
            "index --strict: one or more files failed during reconcile".into(),
        ));
    }
    Ok(())
}

/// Canonicalize `path` and confirm it names a file or directory, mapping
/// a missing/invalid path to a usage error (`EX_USAGE`) rather than the
/// bare `Error::Io` a raw `fs::canonicalize` failure would produce.
fn validate_input_path(path: &Path) -> Result<PathBuf> {
    let canonical = fs::canonicalize(path).map_err(|e| {
        Error::Usage(format!(
            "index: cannot resolve path `{}`: {e}",
            path.display()
        ))
    })?;
    let meta = fs::metadata(&canonical)?;
    if !meta.is_file() && !meta.is_dir() {
        return Err(Error::Usage(format!(
            "index: path `{}` is neither a file nor a directory",
            canonical.display()
        )));
    }
    Ok(canonical)
}

/// Infer a repo label from the nearest enclosing git worktree's basename,
/// or `None` when `canonical` is not inside one. Discovery starts from
/// the containing directory for a file source, and from the root itself
/// for a directory source.
fn infer_repo_label(canonical: &Path) -> Option<String> {
    let start = if canonical.is_dir() {
        canonical
    } else {
        canonical.parent()?
    };
    let repo = git2::Repository::discover(start).ok()?;
    let workdir = repo.workdir()?;
    workdir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
}

/// Discover every candidate under `entry`, index each one, then tombstone
/// any previously-known file the walk no longer saw (spec: "Index
/// lifecycle and freshness", step 5).
fn reconcile_source(
    conn: &mut Connection,
    entry: &SourceEntry,
    max_file_bytes: u64,
    paths: &Paths,
) -> Result<SourceReport> {
    let candidates = discover::discover(&entry.canonical_path, entry.kind, &paths.memories_dir());
    let mut report = SourceReport {
        source_id: entry.id.to_string(),
        canonical_path: entry.canonical_path.to_string_lossy().into_owned(),
        repo: entry.repo.clone(),
        indexed: 0,
        unchanged: 0,
        too_large: 0,
        unsupported: 0,
        ignored: 0,
        removed: 0,
        errors: Vec::new(),
    };
    let mut seen = HashSet::with_capacity(candidates.len());
    for c in &candidates {
        seen.insert(c.relative_path.to_string_lossy().into_owned());
        let outcome = writer::update_file(
            conn,
            entry.id.as_str(),
            entry.repo.as_deref(),
            c,
            &entry.canonical_path,
            max_file_bytes,
        )?;
        record_outcome(&mut report, c, outcome);
    }
    report.removed = writer::reconcile_deletions(conn, entry.id.as_str(), &seen)?;
    Ok(report)
}

/// Fold one candidate's [`UpdateOutcome`] into `report`'s counters,
/// appending a [`FileError`] diagnostic for the two strict-triggering
/// outcomes (AC-7: corrupt/unreadable/oversized).
fn record_outcome(report: &mut SourceReport, c: &Candidate, outcome: UpdateOutcome) {
    let path = c.relative_path.to_string_lossy().into_owned();
    match outcome {
        UpdateOutcome::Unchanged => report.unchanged += 1,
        UpdateOutcome::Indexed { .. } => report.indexed += 1,
        UpdateOutcome::TooLarge => {
            report.too_large += 1;
            report.errors.push(FileError {
                path,
                kind: "too_large",
                message: "file exceeds indexing.max_file_bytes".into(),
            });
        }
        UpdateOutcome::Unsupported => report.unsupported += 1,
        UpdateOutcome::Ignored => report.ignored += 1,
        UpdateOutcome::Error(message) => {
            report.errors.push(FileError {
                path,
                kind: "error",
                message,
            });
        }
    }
}

/// Emit the reconcile report: a single JSON object under `--json`, else a
/// per-source summary line plus a `warning:` line per diagnostic.
fn emit(json_flag: bool, output: &Output) -> Result<()> {
    if json_flag {
        json::write(output)?;
        return Ok(());
    }
    let mut out = std::io::stdout().lock();
    for r in &output.sources {
        writeln!(out, "source {} ({})", r.source_id, r.canonical_path)?;
        writeln!(
            out,
            "  indexed={} unchanged={} too_large={} unsupported={} removed={}",
            r.indexed, r.unchanged, r.too_large, r.unsupported, r.removed
        )?;
        for e in &r.errors {
            tty::warning(&format!("{}: {} ({})", e.path, e.message, e.kind))?;
        }
    }
    Ok(())
}
