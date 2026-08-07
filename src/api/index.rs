//! `api::index::{Request, run}` — shared middle of `comemory index
//! <PATH>...` / `POST /api/v1/sources`: register sources and run their
//! synchronous initial reconcile. Moved out of `cli::index::run`.
//!
//! `Request.path` (not `paths`) matches the clap arg id, for AC-12.
//! `Request.strict` is accepted for AC-12 but never enforced here: a job
//! `Err` would discard the successful partial [`Output`], while
//! `Output.sources[].errors` already carries every diagnostic —
//! `cli::index::run` applies the strict-triggered error itself, after
//! calling [`run`] (AC-13). Path containment is the caller's job.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::api::Ctx;
use crate::document::writer::{self, UpdateOutcome};
use crate::prelude::*;
use crate::source::SourceEntry;
use crate::source::discover::{self, Candidate};
use crate::source::mirror;
use crate::source::registry::Registry;

/// `comemory index <PATH>...` / `POST /api/v1/sources` request.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// One or more files or directories to register as document sources.
    /// Field name matches the clap arg id `path` (AC-12), not the more
    /// natural plural `paths` — see the module doc.
    pub path: Vec<String>,
    /// Repository label attached to every document under each registered
    /// source. Defaults to the basename of the nearest enclosing git
    /// worktree, when one exists.
    #[serde(default)]
    pub repo: Option<String>,
    /// Accepted for parity with `--strict` but not enforced here — see the
    /// module doc's "strict decision".
    #[serde(default)]
    pub strict: bool,
}

/// One per-file diagnostic surfaced in a [`SourceReport`].
#[derive(Serialize, Debug)]
pub struct FileError {
    /// Path relative to the source root.
    pub path: String,
    /// `"too_large"` or `"error"`.
    pub kind: &'static str,
    /// The typed diagnostic message.
    pub message: String,
}

/// Reconcile outcome for one registered source.
#[derive(Serialize, Debug)]
pub struct SourceReport {
    /// The registered source's stable id.
    pub source_id: String,
    /// Absolute, symlink-resolved path this entry roots.
    pub canonical_path: String,
    /// Repository label, if any.
    pub repo: Option<String>,
    /// Files newly indexed (or re-indexed after a content change).
    pub indexed: usize,
    /// Files whose fingerprint matched an already-indexed row.
    pub unchanged: usize,
    /// Files that exceeded `indexing.max_file_bytes`.
    pub too_large: usize,
    /// Files whose format is not indexable.
    pub unsupported: usize,
    /// Files skipped by the ignore-file rules.
    pub ignored: usize,
    /// Previously-indexed files no longer seen by this walk.
    pub removed: usize,
    /// Per-file diagnostics (`too_large` + `error` outcomes).
    pub errors: Vec<FileError>,
}

/// Full `comemory index` / `POST /api/v1/sources` report: one
/// [`SourceReport`] per registered path, in invocation order.
#[derive(Serialize, Debug)]
pub struct Output {
    /// One report per `req.path` entry, in order.
    pub sources: Vec<SourceReport>,
}

/// Register every `req.path` entry and run its synchronous reconcile. See
/// the module doc for why `req.strict` is accepted but never enforced
/// here.
pub fn run(ctx: &mut Ctx<'_>, req: Request) -> Result<Output> {
    let cfg = ctx.cfg;
    let paths = ctx.paths;
    let registry = Registry::new(paths.clone());
    let conn = ctx.conn()?;

    let mut reports = Vec::with_capacity(req.path.len());
    for raw in &req.path {
        let raw_path = PathBuf::from(raw);
        let canonical = validate_input_path(&raw_path)?;
        let repo = req.repo.clone().or_else(|| infer_repo_label(&canonical));
        let entry = registry.register(&raw_path, repo)?;
        // Reconcile the SQLite mirror before indexing this entry so its
        // `source_roots` row (the FK parent of `source_files`) exists.
        mirror::reconcile(conn, &registry.load()?)?;
        let report = reconcile_source(conn, &entry, cfg.indexing.max_file_bytes, paths)?;
        reports.push(report);
    }
    Ok(Output { sources: reports })
}

/// Canonicalize `path` and confirm it names a file or directory, mapping
/// a missing/invalid path to a usage error (`EX_USAGE`) rather than the
/// bare `Error::Io` a raw `fs::canonicalize` failure would produce.
fn validate_input_path(path: &Path) -> Result<PathBuf> {
    let canonical = std::fs::canonicalize(path).map_err(|e| {
        Error::Usage(format!(
            "index: cannot resolve path `{}`: {e}",
            path.display()
        ))
    })?;
    let meta = std::fs::metadata(&canonical)?;
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
    paths: &crate::config::Paths,
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

#[cfg(test)]
#[path = "tests/index.rs"]
mod tests;
