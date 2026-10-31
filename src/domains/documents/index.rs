//! `domains::documents::index::{Request, run}` — shared middle of
//! `comemory index <PATH>...` / `POST /api/v1/sources`: register sources and run their
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

use serde::{Deserialize, Serialize};

use crate::domains::documents::document::deletions;
use crate::domains::documents::document::writer::{self, UpdateOutcome};
use crate::domains::documents::source::SourceEntry;
use crate::domains::documents::source::discover::{self, Candidate};
use crate::domains::documents::source::mirror;
use crate::domains::documents::source::registry::Registry;
use crate::prelude::*;
use crate::store::Connection;
use crate::utilities::context::Ctx;
use crate::utilities::progress::ProgressSink;

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
    /// Previously-indexed files no longer seen by this walk. Always `0` when
    /// [`Self::walk_complete`] is false — see the note there.
    pub removed: usize,
    /// Chunks newly embedded for the configured document provider.
    pub embedded: usize,
    /// Chunks whose embedding failed or became stale during inference.
    pub embedding_failed: usize,
    /// Chunks without a vector for the configured provider (or all chunks when unset).
    pub embedding_pending: usize,
    /// Whether the discovery walk read every entry under the source.
    ///
    /// `false` means at least one entry was unreadable, so this run made no
    /// deletions at all: a short candidate list would otherwise tombstone
    /// files that still exist, and a document tombstone is replicated to every
    /// peer. Fix the permission and index again.
    pub walk_complete: bool,
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
    run_with_progress(ctx, req, None)
}

/// Reconcile sources while reporting progress and honoring cancellation.
pub fn run_with_progress(
    ctx: &mut Ctx<'_>,
    req: Request,
    sink: Option<&dyn ProgressSink>,
) -> Result<Output> {
    run_with_mode(ctx, req, sink, false)
}

/// Reconcile sources with optional full document-vector refresh.
pub fn run_with_mode(
    ctx: &mut Ctx<'_>,
    req: Request,
    sink: Option<&dyn ProgressSink>,
    full: bool,
) -> Result<Output> {
    let cfg = ctx.cfg;
    let paths = ctx.paths;
    let registry = Registry::new(paths.clone());
    let conn = ctx.conn()?;

    let mut reports = Vec::with_capacity(req.path.len());
    for raw in &req.path {
        check_cancelled(sink)?;
        let raw_path = PathBuf::from(raw);
        let canonical = validate_input_path(&raw_path)?;
        let repo = req.repo.clone().or_else(|| infer_repo_label(&canonical));
        let entry = registry.register(&raw_path, repo)?;
        // Reconcile the SQLite mirror before indexing this entry so its
        // `source_roots` row (the FK parent of `source_files`) exists.
        mirror::reconcile(conn, &registry.load()?)?;
        let mut report = reconcile_source(conn, &entry, cfg.indexing.max_file_bytes, paths, sink)?;
        embed_source(conn, cfg, &mut report, sink, full)?;
        reports.push(report);
    }
    Ok(Output { sources: reports })
}

fn embed_source(
    conn: &Connection,
    cfg: &crate::config::Config,
    report: &mut SourceReport,
    sink: Option<&dyn ProgressSink>,
    full: bool,
) -> Result<()> {
    let source = &report.source_id;
    if let Some(cmd) = cfg
        .document_embed_cmd
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        let hint = cfg.document_embed_hint.as_deref();
        let (embedded, errors) =
            super::chunk_embedding::reconcile(conn, source, cmd, hint, sink, full)?;
        report.embedded = embedded;
        report.embedding_failed = errors.len();
        let model = super::chunk_embedding::provider_id(cmd, hint);
        report.embedding_pending =
            crate::store::document_vectors::pending(conn, source, &model, false)?.len();
        report
            .errors
            .extend(errors.into_iter().map(|message| FileError {
                path: report.canonical_path.clone(),
                kind: "embedding",
                message,
            }));
    } else {
        report.embedding_pending =
            crate::store::document_vectors::pending(conn, source, "", true)?.len();
    }
    Ok(())
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

/// Infer a repo label from the enclosing git repository — its main
/// worktree's basename ([`crate::domains::code::git_utils::repo_label`]), so a source
/// registered from a linked worktree files under the same label as one
/// registered from the main checkout — or `None` when `canonical` is not
/// inside one. Discovery starts from the containing directory for a file
/// source, and from the root itself for a directory source.
fn infer_repo_label(canonical: &Path) -> Option<String> {
    let start = if canonical.is_dir() {
        canonical
    } else {
        canonical.parent()?
    };
    crate::domains::code::git_utils::repo_label_at(start)
}

/// Discover every candidate under `entry`, index each one, then tombstone
/// any previously-known file the walk no longer saw (spec: "Index
/// lifecycle and freshness", step 5).
fn reconcile_source(
    conn: &mut Connection,
    entry: &SourceEntry,
    max_file_bytes: u64,
    paths: &crate::config::Paths,
    sink: Option<&dyn ProgressSink>,
) -> Result<SourceReport> {
    check_cancelled(sink)?;
    let data_dir =
        std::fs::canonicalize(paths.data_dir()).unwrap_or_else(|_| paths.data_dir().to_path_buf());
    let discovered = discover::discover(&entry.canonical_path, entry.kind, &data_dir);
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
        embedded: 0,
        embedding_failed: 0,
        embedding_pending: 0,
        walk_complete: discovered.complete,
        errors: Vec::new(),
    };
    // What the walk saw, independent of what indexing each file does with it —
    // which is why the deletion pass can run first.
    let seen: HashSet<String> = discovered
        .candidates
        .iter()
        .map(|c| c.relative_path.to_string_lossy().into_owned())
        .collect();
    // Deletions BEFORE writes, so a renamed document journals the old path's
    // tombstone ahead of the new path's revision and never has both rows at
    // once. Only an authoritative walk's absences mean anything, though:
    // `reconcile_deletions` requires `seen` to be the FULL set of paths the
    // walk saw, and a walk that skipped an unreadable entry did not produce
    // one — reconciling it would tombstone files that still exist, and a
    // document tombstone is journalled for every peer to act on. A per-file
    // failure is NOT incompleteness: the path is in `seen` either way, so an
    // unreadable single FILE keeps its rows.
    if discovered.complete {
        report.removed = deletions::reconcile_deletions(conn, entry.id.as_str(), &seen)?;
    } else {
        tracing::warn!(
            source_id = %entry.id,
            "discovery walk was incomplete; keeping every known file"
        );
    }
    for (i, c) in discovered.candidates.iter().enumerate() {
        check_cancelled(sink)?;
        let outcome = writer::update_file(
            conn,
            entry.id.as_str(),
            entry.repo.as_deref(),
            c,
            &entry.canonical_path,
            max_file_bytes,
        )?;
        record_outcome(&mut report, c, outcome);
        if let Some(sink) = sink {
            sink.on_progress((i + 1) as u64, discovered.candidates.len() as u64);
        }
    }
    Ok(report)
}

fn check_cancelled(sink: Option<&dyn ProgressSink>) -> Result<()> {
    if sink.is_some_and(ProgressSink::is_cancelled) {
        return Err(Error::Cancelled);
    }
    Ok(())
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
